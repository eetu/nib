//! SQLite persistence (Phase C) via `sqlx` — the store of record for users + projects. A
//! project's `svg` column holds the document source. Queries are runtime-checked (`query_as`), so
//! there's no build-time database dependency.

use std::str::FromStr;

use serde::Serialize;
use sqlx::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

/// The dev token if `NIB_DEV_TOKEN` is unset — so `just dev` + a local MCP client work with zero
/// setup. In any real deployment set `NIB_DEV_TOKEN` (or, later, real per-user tokens).
pub const DEV_TOKEN_DEFAULT: &str = "nib-dev-token";

#[derive(Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub token: String,
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

/// Ensure a `developer` user exists with the given token (idempotent — dev bootstrap).
pub async fn ensure_dev_user(pool: &SqlitePool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into users (name, token) values ('developer', ?) on conflict(token) do nothing",
    )
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn user_by_token(pool: &SqlitePool, token: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("select id, name, token from users where token = ?")
        .bind(token)
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
