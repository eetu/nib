//! OIDC authorization-code-with-PKCE flow against an external issuer (kanidm in production).
//! Ported from the sibling apps' `oidc.rs` (represent/scribe/chat share this shape verbatim) —
//! keeping it identical means the family's hard-won details travel with it.
//!
//! The browser never sees access or ID tokens — they live for the duration of the callback
//! exchange only. After validation we drop them and persist just `{ sub, email }` in the signed
//! session cookie.

use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use thiserror::Error;
use url::Url;

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::OidcSettings;

#[derive(Error, Debug)]
pub enum OidcError {
    #[error("oidc init failed: {0}")]
    Init(String),
    #[error("oidc discovery failed: {0}")]
    Discover(String),
    #[error("oidc exchange failed: {0}")]
    Exchange(String),
    #[error("oidc id_token missing or invalid: {0}")]
    Token(String),
    #[error("http client build failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),
}

#[derive(Clone)]
pub struct OidcContext {
    provider: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_url: RedirectUrl,
    http: reqwest::Client,
}

impl OidcContext {
    pub async fn discover(s: &OidcSettings) -> Result<Self, OidcError> {
        // Disable redirects on the OIDC HTTP client — required SSRF protection per
        // openidconnect-rs guidance, since metadata + token endpoints must not follow
        // arbitrary redirects.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // Bounded so a hung issuer can't stall discovery — which runs on the login path,
            // not just at boot (see `OidcLazy`).
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()?;

        let issuer = IssuerUrl::new(s.issuer.clone())
            .map_err(|e| OidcError::Init(format!("issuer url: {e}")))?;

        let provider = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| OidcError::Discover(e.to_string()))?;

        Ok(Self {
            provider,
            client_id: ClientId::new(s.client_id.clone()),
            client_secret: ClientSecret::new(s.client_secret.clone()),
            redirect_url: RedirectUrl::new(s.redirect_url.clone())?,
            http,
        })
    }

    fn client(
        &self,
    ) -> CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    > {
        CoreClient::from_provider_metadata(
            self.provider.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect_url.clone())
    }

    /// Build the authorize URL and the values that need to round-trip via the handshake cookie
    /// until the callback fires.
    pub fn authorize(&self) -> Authorize {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (auth_url, csrf, nonce) = self
            .client()
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("openid".into()))
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        Authorize {
            url: auth_url,
            csrf,
            nonce,
            pkce_verifier,
        }
    }

    /// Exchange the authorization code for tokens and validate the ID token. Returns the verified
    /// subject + email + display name (preferring the email claim, falling back to
    /// `preferred_username@local` for issuers that omit it — kanidm omits `email` when a person
    /// carries no `mail` attribute).
    pub async fn exchange(
        &self,
        code: &str,
        pkce_verifier: PkceCodeVerifier,
        nonce: Nonce,
    ) -> Result<VerifiedClaims, OidcError> {
        let client = self.client();
        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::Exchange(e.to_string()))?
            .set_pkce_verifier(pkce_verifier)
            .request_async(&self.http)
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;

        let id_token = token_response
            .id_token()
            .ok_or_else(|| OidcError::Token("server did not return id_token".into()))?;

        let claims = id_token
            .claims(&client.id_token_verifier(), &nonce)
            .map_err(|e| OidcError::Token(e.to_string()))?;

        let sub = claims.subject().as_str().to_string();
        let preferred_username = claims
            .preferred_username()
            .map(|n| n.as_str().to_string())
            .or_else(|| {
                claims
                    .name()
                    .and_then(|n| n.get(None))
                    .map(|n| n.as_str().to_string())
            });
        let email = claims
            .email()
            .map(|e| e.as_str().to_string())
            .or_else(|| preferred_username.as_ref().map(|u| format!("{u}@local")))
            .unwrap_or_else(|| format!("{sub}@local"));
        let name = preferred_username.unwrap_or_else(|| email.clone());

        // Token response is dropped here — we never store access/refresh tokens. The session
        // cookie holds only `{ sub, email }`.
        drop(token_response);

        Ok(VerifiedClaims { sub, email, name })
    }
}

pub struct Authorize {
    pub url: Url,
    pub csrf: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
}

pub struct VerifiedClaims {
    pub sub: String,
    pub email: String,
    pub name: String,
}

/// Lazily-discovered OIDC provider with on-demand retry.
///
/// Discovery is NOT done at boot. The issuer (kanidm) may boot concurrently with this process; a
/// one-shot boot discovery that failed would leave the app serving "auth unavailable" forever,
/// since the IaC only restarts nib on a config change — and the health endpoint would still
/// return 200, so no orchestrator would restart it either.
///
/// Instead every auth-needing caller routes through [`OidcLazy::ctx`]. The first call (or the next
/// after the issuer comes up) discovers and caches, so a retry self-heals with no restart. The
/// write lock single-flights concurrent retries so a burst makes at most one discovery call.
///
/// It is also what makes a **fleet bootstrap** safe: until kanidm is running and has issued nib's
/// client secret there are no usable `OIDC_*`, and nib boots healthy without them instead of
/// crash-looping on an issuer that isn't there yet.
pub struct OidcLazy {
    settings: Option<OidcSettings>,
    cached: RwLock<Option<Arc<OidcContext>>>,
}

impl OidcLazy {
    pub fn new(settings: Option<OidcSettings>) -> Self {
        Self {
            settings,
            cached: RwLock::new(None),
        }
    }

    /// Whether OIDC is configured at all (env vars present). `false` = a dev-auth-only deploy;
    /// there's nothing to discover.
    pub fn is_configured(&self) -> bool {
        self.settings.is_some()
    }

    /// Resolve the OIDC context, discovering (or re-discovering) on demand. Returns `None` when
    /// OIDC isn't configured, or when discovery is still failing (issuer down) — the caller should
    /// surface a retryable 503.
    pub async fn ctx(&self) -> Option<Arc<OidcContext>> {
        // Fast path: already discovered.
        if let Some(ctx) = self.cached.read().await.as_ref() {
            return Some(ctx.clone());
        }
        // Nothing to retry if OIDC isn't configured.
        let settings = self.settings.as_ref()?;
        let mut guard = self.cached.write().await;
        // Double-check: another task may have discovered while we waited.
        if let Some(ctx) = guard.as_ref() {
            return Some(ctx.clone());
        }
        match OidcContext::discover(settings).await {
            Ok(ctx) => {
                tracing::info!(issuer = %settings.issuer, "oidc provider discovered");
                let ctx = Arc::new(ctx);
                *guard = Some(ctx.clone());
                Some(ctx)
            }
            Err(e) => {
                tracing::warn!(issuer = %settings.issuer, error = %e, "oidc discovery failed; will retry on the next login");
                None
            }
        }
    }
}
