//! SQLite persistence (Phase C) via `sqlx` — the store of record for users + projects. A
//! project's `svg` column holds the document source. Queries are runtime-checked (`query_as`), so
//! there's no build-time database dependency.

use std::str::FromStr;
use std::time::Duration;

use serde::Serialize;
use sqlx::FromRow;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

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
    /// The first characters of the bearer token, in the clear — enough to tell one token from
    /// another in the UI, worth nothing as a credential. The token itself is only ever held as a
    /// hash, so nothing can hand it back after the moment it was minted.
    pub token_hint: String,
    /// `None` only for rows predating OIDC (and never in a fresh deployment).
    pub email: Option<String>,
}

/// The columns a `User` is built from. Spelled once: every lookup must select the same set, and
/// `token` is deliberately not among them.
const USER_COLS: &str = "id, name, token_hint, email";

/// Mint a fresh bearer token. Prefixed so it's recognisable in a config file or a paste.
pub fn mint_token() -> String {
    format!("nib_{}", crate::config::random_hex(32))
}

/// A token's stored form. See `migrations/0006` for why this is a bare SHA-256 and not a KDF.
pub fn hash_token(token: &str) -> String {
    use std::fmt::Write;

    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The part of a token safe to show. `nib_` plus 8 of the 64 hex characters: enough to recognise
/// which token a client is configured with, 56 characters short of being usable.
pub fn token_hint(token: &str) -> String {
    token.chars().take(12).collect()
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
    /// How many times this project's document has been replaced wholesale — see
    /// `migrations/0005`. A client echoes it back as `If-Match` on `PUT` so a replacement that
    /// would land on top of one it never saw is refused instead of silently winning.
    pub generation: i64,
}

/// Open (creating if missing) the SQLite database at `url` and run migrations.
///
/// The journal mode is the load-bearing setting. SQLite's default rollback journal takes a lock
/// that excludes *readers* for the whole write, so with a pool of connections a co-editing session
/// — the human's `PUT`, the LLM's op, and a page load all landing together — produces
/// `database is locked` under ordinary use rather than under stress. WAL lets readers run
/// alongside the writer, and `busy_timeout` makes the one remaining case (two writers) wait its
/// turn instead of failing instantly.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL is the documented companion to WAL: durable across a process crash, and only a
        // very recent commit is at risk if the machine loses power. FULL fsyncs every commit,
        // which on the Pi's SD card is the difference between an edit landing and a visible stall.
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
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
    let token = mint_token();
    sqlx::query_as::<_, User>(&format!(
        // `retired_plaintext_token` is migration 0006's renamed corpse of the old column: kept
        // because dropping it would mean rebuilding the table, still `not null unique`, so every
        // row needs a distinct meaningless value. Nothing ever reads it.
        "insert into users (name, retired_plaintext_token, token_hash, token_hint, sub, email) \
         values (?, lower(hex(randomblob(16))), ?, ?, ?, ?) \
         on conflict(sub) do update set email = excluded.email, name = excluded.name \
         returning {USER_COLS}",
    ))
    .bind(name)
    .bind(hash_token(&token))
    .bind(token_hint(&token))
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
    // Whoever else holds this token gets a fresh (unguessable) one. Matched on the hash, since
    // that is now the only form stored.
    sqlx::query(
        "update users set token_hash = lower(hex(randomblob(32))), token_hint = '', \
         token_rotated_at = datetime('now') where token_hash = ? and id <> ?",
    )
    .bind(hash_token(token))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update users set token_hash = ?, token_hint = ?, \
         token_rotated_at = datetime('now') where id = ?",
    )
    .bind(hash_token(token))
    .bind(token_hint(token))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(User {
        token_hint: token_hint(token),
        ..user
    })
}

/// Replace a user's bearer token. Every surface re-resolves the token per request, so the old one
/// stops working immediately — except on already-established WebSockets, which authenticate at
/// connect time only.
pub async fn set_token(pool: &SqlitePool, user_id: i64, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update users set token_hash = ?, token_hint = ?, \
         token_rotated_at = datetime('now') where id = ?",
    )
    .bind(hash_token(token))
    .bind(token_hint(token))
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

/// Resolve a presented bearer token. The hash is what's compared, so the database never holds the
/// value an attacker would need — and the comparison stays one indexed read.
pub async fn user_by_token(pool: &SqlitePool, token: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(&format!(
        "select {USER_COLS} from users where token_hash = ?"
    ))
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await
}

/// Look up a user by OIDC subject — the read behind the session cookie.
pub async fn user_by_sub(pool: &SqlitePool, sub: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>(&format!("select {USER_COLS} from users where sub = ?"))
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
        "select id, user_id, name, model, svg, created_at, updated_at, generation \
         from projects where id = ? and user_id = ?",
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

/// Replace a project's document, bumping its `generation` — the write `PUT` makes.
///
/// `expected` is the generation the caller believes it is replacing. `Some(g)` makes the write
/// conditional in **SQL**, so the check and the write are one statement and two callers racing
/// can't both pass it; `None` forces (a caller that never read a generation, which is what every
/// client did before this existed). `Ok(None)` means the row moved on — the caller lost.
pub async fn replace_project(
    pool: &SqlitePool,
    id: i64,
    model: &str,
    svg: &str,
    expected: Option<i64>,
) -> Result<Option<i64>, sqlx::Error> {
    let mut q = sqlx::QueryBuilder::new("update projects set model = ");
    q.push_bind(model)
        .push(", svg = ")
        .push_bind(svg)
        .push(", generation = generation + 1, updated_at = datetime('now') where id = ")
        .push_bind(id);
    if let Some(g) = expected {
        q.push(" and generation = ").push_bind(g);
    }
    q.push(" returning generation");
    q.build_query_scalar::<i64>().fetch_optional(pool).await
}
