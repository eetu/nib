//! The OIDC dance: `/auth/login` → issuer → `/auth/callback` → a signed session cookie.
//!
//! `auth.rs` answers "who is this caller"; this module is how a browser becomes one. Nothing here
//! is reachable from `/mcp` — an MCP client authenticates with its bearer token and never logs in.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, SameSite, SignedCookieJar};
use openidconnect::{Nonce, PkceCodeVerifier};
use serde::Deserialize;

use crate::AppState;
use crate::auth::{OIDC_COOKIE, SESSION_COOKIE};
use crate::db;

#[derive(Deserialize)]
pub struct LoginQuery {
    next: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, msg.into()).into_response()
}

/// Only same-site absolute paths survive, so `?next=` can't be used as an open redirect.
fn sanitize_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") => n.to_string(),
        _ => "/".to_string(),
    }
}

/// Write the session cookie. It carries only the OIDC subject — the profile lives in the DB, so a
/// stolen cookie conveys no more than "this subject is signed in".
fn write_session(jar: SignedCookieJar, sub: &str, secure: bool) -> SignedCookieJar {
    jar.add(
        Cookie::build((SESSION_COOKIE, sub.to_string()))
            .path("/")
            .http_only(true)
            // Lax, not Strict: Strict would drop the cookie on the cross-site redirect back from
            // kanidm, and the user would land signed-out on the page that just signed them in.
            .same_site(SameSite::Lax)
            .secure(secure)
            .build(),
    )
}

/// Start the authorization-code flow.
pub async fn login(
    State(st): State<AppState>,
    jar: SignedCookieJar,
    Query(q): Query<LoginQuery>,
) -> Response {
    let dest = sanitize_next(q.next.as_deref());
    let secure = !st.cfg.dev_auth;

    if st.oidc.is_configured() {
        match st.oidc.ctx().await {
            Some(oidc) => {
                let auth = oidc.authorize();
                // The handshake values round-trip in their own short-lived signed cookie rather
                // than server-side state, so the process stays stateless across restarts.
                let payload = format!(
                    "{}|{}|{}|{}",
                    auth.csrf.secret(),
                    auth.nonce.secret(),
                    auth.pkce_verifier.secret(),
                    dest
                );
                let cookie = Cookie::build((OIDC_COOKIE, payload))
                    .path("/")
                    .http_only(true)
                    .same_site(SameSite::Lax)
                    .secure(secure)
                    .max_age(time::Duration::minutes(10))
                    .build();
                return (jar.add(cookie), Redirect::to(auth.url.as_str())).into_response();
            }
            // Configured but undiscoverable — the issuer is down or still booting. Retryable.
            None if !st.cfg.dev_auth => {
                return err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "auth provider not reachable; retry shortly",
                );
            }
            None => {}
        }
    }

    if st.cfg.dev_auth {
        // No kanidm locally: sign in as the seeded dev identity.
        return (write_session(jar, db::DEV_SUB, false), Redirect::to(&dest)).into_response();
    }

    err(
        StatusCode::BAD_REQUEST,
        "auth not configured. set NIB_DEV_AUTH=1 or all four OIDC_* env vars",
    )
}

/// Finish the flow: verify the handshake, exchange the code, upsert the user, set the session.
pub async fn callback(
    State(st): State<AppState>,
    jar: SignedCookieJar,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(handshake) = jar.get(OIDC_COOKIE).map(|c| c.value().to_string()) else {
        return err(StatusCode::BAD_REQUEST, "missing oidc handshake cookie");
    };
    // Drop the handshake cookie regardless of outcome — a replay is never useful, and leaving
    // partial state around is worse than making the user start over.
    let jar = jar.remove(Cookie::build((OIDC_COOKIE, "")).path("/").build());

    // splitn(4): an email or a `next` containing '|' can't corrupt the parse.
    let parts: Vec<&str> = handshake.splitn(4, '|').collect();
    let [csrf, nonce, pkce, dest] = parts[..] else {
        return err(StatusCode::BAD_REQUEST, "malformed handshake cookie");
    };
    if csrf != q.state {
        tracing::warn!("oidc state mismatch — possible csrf");
        return err(StatusCode::BAD_REQUEST, "state mismatch");
    }

    let Some(oidc) = st.oidc.ctx().await else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth provider not reachable; retry shortly",
        );
    };

    let claims = match oidc
        .exchange(
            &q.code,
            PkceCodeVerifier::new(pkce.to_string()),
            Nonce::new(nonce.to_string()),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "oidc code exchange failed");
            return err(StatusCode::BAD_GATEWAY, "oidc exchange failed");
        }
    };

    // First login for this subject mints the user's bearer token.
    let user = match db::resolve_user(&st.pool, &claims.sub, &claims.email, &claims.name).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "resolving oidc user failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };
    tracing::info!(user = user.id, "signed in");

    let dest = sanitize_next(Some(dest));
    (
        write_session(jar, &claims.sub, !st.cfg.dev_auth),
        Redirect::to(&dest),
    )
        .into_response()
}

/// Clear the session cookie. The bearer token is untouched — logging out of the browser must not
/// break a configured MCP client.
pub async fn logout(jar: SignedCookieJar) -> Response {
    let jar = jar.remove(Cookie::build((SESSION_COOKIE, "")).path("/").build());
    (jar, StatusCode::NO_CONTENT).into_response()
}

#[cfg(test)]
mod tests {
    use super::sanitize_next;

    #[test]
    fn next_only_accepts_same_site_paths() {
        assert_eq!(sanitize_next(Some("/editor?id=3")), "/editor?id=3");
        // Protocol-relative URLs are the classic open-redirect vector.
        assert_eq!(sanitize_next(Some("//evil.example.com")), "/");
        assert_eq!(sanitize_next(Some("https://evil.example.com")), "/");
        assert_eq!(sanitize_next(None), "/");
    }
}
