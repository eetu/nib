//! Authentication — one resolver, three credentials.
//!
//! Every surface (REST, WebSocket, MCP) used to resolve the caller its own way, with three
//! different failure modes. They now share [`resolve`], and each surface declares which
//! credential **tiers** it accepts:
//!
//! * **cookie** — the signed `nib_session` cookie written by the OIDC callback. The browser's
//!   credential.
//! * **bearer** — `Authorization: Bearer <token>`, the user's personal, rotatable API token. What
//!   an MCP client presents.
//! * **dev** — a synthetic identity under `NIB_DEV_AUTH`, so `just dev` needs no kanidm.
//!
//! Two deliberate asymmetries:
//!
//! * `/api/me` and token rotation are **cookie-only**. A leaked bearer must not be able to read
//!   itself back or mint its replacement — the browser session is what authorizes minting.
//! * `/mcp` is **bearer-only**. It's same-origin with the SPA, so honouring the cookie there would
//!   let any page in the browser drive the whole tool surface (CSRF). This is also what "the MCP
//!   path bypasses SSO" means concretely: no session, just the token.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Key, SignedCookieJar};
use sqlx::SqlitePool;

use crate::AppState;
use crate::db::{self, User};

/// Signed cookie holding the authenticated OIDC subject. The rest of the profile lives in the DB,
/// so nothing here needs to be trusted beyond "this subject logged in".
pub const SESSION_COOKIE: &str = "nib_session";
/// Short-lived signed cookie carrying the in-flight OIDC handshake (`csrf|nonce|pkce|next`).
pub const OIDC_COOKIE: &str = "nib_oidc";

/// Which credentials a surface will accept.
#[derive(Clone, Copy)]
pub struct Tiers {
    pub cookie: bool,
    pub bearer: bool,
}

impl Tiers {
    /// Browser session or API token — the normal REST surface.
    pub const FULL: Tiers = Tiers {
        cookie: true,
        bearer: true,
    };
    /// Browser session only — reading or rotating the API token, and the WebSocket's first try.
    pub const SESSION: Tiers = Tiers {
        cookie: true,
        bearer: false,
    };
}

pub enum AuthError {
    Unauthorized(&'static str),
    Internal,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match self {
            AuthError::Unauthorized(msg) => (
                StatusCode::UNAUTHORIZED,
                [(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_static("Bearer realm=\"nib\""),
                )],
                msg,
            )
                .into_response(),
            // Deliberately opaque: the sqlx error text used to go straight to the client.
            AuthError::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}

/// The `Authorization: Bearer <token>` value, if present.
pub fn bearer(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// The authenticated subject in the signed session cookie, if any.
pub fn session_sub(parts: &Parts, key: &Key) -> Option<String> {
    SignedCookieJar::from_headers(&parts.headers, key.clone())
        .get(SESSION_COOKIE)
        .map(|c| c.value().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve the caller against the tiers this surface accepts. Tried in order: cookie, bearer,
/// then the dev-auth fallback.
pub async fn resolve(state: &AppState, parts: &Parts, tiers: Tiers) -> Result<User, AuthError> {
    if tiers.cookie
        && let Some(sub) = session_sub(parts, &state.cookie_key)
    {
        // The row is looked up, never upserted: the profile was written at callback time, and
        // a cookie for a user who no longer exists should fail, not resurrect them.
        return db::user_by_sub(&state.pool, &sub)
            .await
            .map_err(|_| AuthError::Internal)?
            .ok_or(AuthError::Unauthorized("session no longer valid"));
    }
    if tiers.bearer
        && let Some(token) = bearer(parts)
    {
        return db::user_by_token(&state.pool, &token)
            .await
            .map_err(|_| AuthError::Internal)?
            .ok_or(AuthError::Unauthorized("invalid token"));
    }
    if state.cfg.dev_auth {
        return db::user_by_sub(&state.pool, db::DEV_SUB)
            .await
            .map_err(|_| AuthError::Internal)?
            .ok_or(AuthError::Internal);
    }
    Err(AuthError::Unauthorized(if tiers.cookie {
        "not signed in"
    } else {
        "missing bearer token"
    }))
}

/// Bearer-only resolution for the MCP tools, which hold the raw HTTP parts rather than an
/// extractor. Falls back to the dev identity so a local MCP client works under `just dev`.
pub async fn mcp_user(pool: &SqlitePool, dev_auth: bool, parts: &Parts) -> Option<User> {
    if let Some(token) = bearer(parts) {
        return db::user_by_token(pool, &token).await.ok().flatten();
    }
    if dev_auth {
        return db::user_by_sub(pool, db::DEV_SUB).await.ok().flatten();
    }
    None
}

/// Axum extractor: a signed-in user, by session cookie **or** bearer token.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve(state, parts, Tiers::FULL).await.map(AuthUser)
    }
}

/// Axum extractor: a browser session specifically — a bearer token is *not* enough. Guards the
/// endpoints that reveal or replace the bearer token itself.
pub struct AuthSession(pub User);

impl FromRequestParts<AppState> for AuthSession {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve(state, parts, Tiers::SESSION).await.map(AuthSession)
    }
}
