//! Token auth (Phase C). Every backend surface (REST, WebSocket, MCP) authenticates with a
//! per-user opaque bearer token: `Authorization: Bearer <token>`. No accounts/login yet — a seeded
//! `developer` user covers dev; the token is what an MCP client presents to act as that user.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sqlx::SqlitePool;

use crate::AppState;
use crate::db::{self, User};

/// The `Authorization: Bearer <token>` value, if present.
pub fn bearer(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
}

/// Resolve the authenticated user from a request's bearer token — used by the MCP tools, which get
/// the raw HTTP parts from the request context.
pub async fn user_from_parts(pool: &SqlitePool, parts: &Parts) -> Option<User> {
    let token = bearer(parts)?;
    db::user_by_token(pool, &token).await.ok().flatten()
}

/// Axum extractor: requires a valid bearer token, yielding the authenticated [`User`].
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token =
            bearer(parts).ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".to_string()))?;
        let user = db::user_by_token(&state.pool, &token)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::UNAUTHORIZED, "invalid token".to_string()))?;
        Ok(AuthUser(user))
    }
}
