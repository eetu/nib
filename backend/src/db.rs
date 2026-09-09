//! SQLite persistence (Phase C) via `sqlx` — the store of record for users + projects. A
//! project's `svg` column holds the document source. Queries are runtime-checked (`query_as`), so
//! there's no build-time database dependency.

use std::str::FromStr;

use serde::Serialize;
use sqlx::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

/// The dev token if `NIB_DEV_TOKEN` is unset — so `just dev` + a local MCP client work with zero
/// setup. Seeded **only** under `NIB_DEV_AUTH`; a production deploy never creates this user, and
/// real users mint their own token on first OIDC login.
pub const DEV_TOKEN_DEFAULT: &str = "nib-dev-token";

/// The OIDC subject of the synthetic dev identity. Sharing one `sub` between the dev-auth cookie
/// tier and the seeded dev token means both resolve to the *same* row — otherwise `just dev` would
/// silently give the browser and the MCP client two different users with two sets of projects.
pub const DEV_SUB: &str = "dev";

#[derive(Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub token: String,
    /// `None` only for rows predating OIDC (and never in a fresh deployment).
    pub email: Option<String>,
}

/// Mint a fresh bearer token. Prefixed so it's recognisable in a config file or a paste.
pub fn mint_token() -> String {
    format!("nib_{}", crate::config::random_hex(32))
}

/// A project row minus its (potentially large) `svg` — for listings.
#[derive(Clone, FromRow, Serialize)]
pub struct ProjectMeta {
    pub id: i64,
    pub name: String,
    pub updated_at: String,
}

/// A full project row. `model` is the native document model JSON (the source of truth); `svg` is a
/// cached canonical export (empty for a brand-new project until first edited). A freshly-created
/// project has an empty `model` until its session first opens (which imports `svg` → model).
#[derive(Clone, FromRow, Serialize)]
pub struct Project {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub model: String,
    pub svg: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Open (creating if missing) the SQLite database at `url` and run migrations.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
    let pool = SqlitePoolOptions::new().connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Look up (or create) the user behind a verified OIDC subject, refreshing the mutable profile
/// bits. Identity is the `sub` — an issuer-stable id — so a rename or a changed email address
/// follows the same account instead of forking a new one.
///
/// A brand-new user gets a freshly minted bearer token here, at creation: that's the credential
/// the UI shows in Settings and the user pastes into an MCP client.
pub async fn resolve_user(
    pool: &SqlitePool,
    sub: &str,
    email: &str,
    name: &str,
) -> Result<User, sqlx::Error> {
    sqlx::query_as::<_, User>(
        "insert into users (name, token, sub, email) values (?, ?, ?, ?) \
         on conflict(sub) do update set email = excluded.email, name = excluded.name \
         returning id, name, token, email",
    )
    .bind(name)
    .bind(mint_token())
    .bind(sub)
    .bind(email)
    .fetch_one(pool)
    .await
}

/// Ensure the synthetic `developer` user exists and carries `token` (idempotent — dev bootstrap).
/// Only called under `NIB_DEV_AUTH`.
///
/// It *takes* the token rather than assuming it's free: `token` is unique, and a database that
/// predates OIDC has a row holding the very token this seeds (see migration 0004). Whoever else has
/// it gets a fresh one — under dev auth the token is how the developer authenticates, so the seed
/// has to win, and the alternative is what used to happen: a unique-constraint error that panics
/// the process before it can serve anything.
pub async fn ensure_dev_user(pool: &SqlitePool, token: &str) -> Result<User, sqlx::Error> {
    let user = resolve_user(pool, DEV_SUB, "dev@localhost", "developer").await?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "update users set token = 'nib_' || lower(hex(randomblob(32))), \
         token_rotated_at = datetime('now') where token = ? and id <> ?",
    )
    .bind(token)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update users set token = ?, token_rotated_at = datetime('now') where id = ?")
        .bind(token)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(User {
        token: token.to_string(),
        ..user
    })
}

/// Replace a user's bearer token. Every surface re-resolves the token per request, so the old one
/// stops working immediately — except on already-established WebSockets, which authenticate at
/// connect time only.
pub async fn set_token(pool: &SqlitePool, user_id: i64, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("update users set token = ?, token_rotated_at = datetime('now') where id = ?")
        .bind(token)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mint a new bearer for `user_id` and return it.
pub async fn rotate_token(pool: &SqlitePool, user_id: i64) -> Result<String, sqlx::Error> {
    let token = mint_token();
    set_token(pool, user_id, &token).await?;
    Ok(token)
}

pub async fn user_by_token(pool: &SqlitePool, token: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("select id, name, token, email from users where token = ?")
        .bind(token)
        .fetch_optional(pool)
        .await
}

/// Look up a user by OIDC subject — the read behind the session cookie.
pub async fn user_by_sub(pool: &SqlitePool, sub: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("select id, name, token, email from users where sub = ?")
        .bind(sub)
        .fetch_optional(pool)
        .await
}

pub async fn list_projects(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<ProjectMeta>, sqlx::Error> {
    sqlx::query_as::<_, ProjectMeta>(
        "select id, name, updated_at from projects where user_id = ? order by updated_at desc",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn get_project(
    pool: &SqlitePool,
    user_id: i64,
    id: i64,
) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>(
        "select id, user_id, name, model, svg, created_at, updated_at from projects where id = ? and user_id = ?",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn create_project(
    pool: &SqlitePool,
    user_id: i64,
    name: &str,
    svg: &str,
) -> Result<i64, sqlx::Error> {
    let res = sqlx::query("insert into projects (user_id, name, svg) values (?, ?, ?)")
        .bind(user_id)
        .bind(name)
        .bind(svg)
        .execute(pool)
        .await?;
    Ok(res.last_insert_rowid())
}

/// Rename a project. Ownership-scoped in the statement itself, so a non-owner's rename simply
/// affects no rows — indistinguishable from a missing project, which is what the caller reports.
pub async fn rename_project(
    pool: &SqlitePool,
    user_id: i64,
    id: i64,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "update projects set name = ?, updated_at = datetime('now') where id = ? and user_id = ?",
    )
    .bind(name)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Delete a project. Ownership-scoped the same way as [`rename_project`].
pub async fn delete_project(pool: &SqlitePool, user_id: i64, id: i64) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("delete from projects where id = ? and user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Persist a project's native model (source of truth) plus its cached SVG export, in one write.
pub async fn update_project(
    pool: &SqlitePool,
    id: i64,
    model: &str,
    svg: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update projects set model = ?, svg = ?, updated_at = datetime('now') where id = ?",
    )
    .bind(model)
    .bind(svg)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
