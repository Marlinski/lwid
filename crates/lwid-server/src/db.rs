//! Database access layer for LookWhatIDid.
//!
//! Provides a thin async wrapper over SQLite via `sqlx`, covering users,
//! sessions, and project ownership.  All queries use runtime-checked SQL
//! (no compile-time `query!` macros) so that `DATABASE_URL` is not required
//! at build time.

use chrono::Utc;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    SqlitePool,
};
use std::str::FromStr;

// ── Connection pool ──────────────────────────────────────────────────────────

/// Open (or create) the SQLite database at `db_path`, run pending migrations,
/// and return a connection pool ready for use.
pub async fn init_pool(db_path: &std::path::Path) -> Result<SqlitePool, sqlx::Error> {
    // Ensure the parent directory exists.
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| sqlx::Error::Io(e))?;
    }

    let opts = SqliteConnectOptions::from_str(
        &format!("sqlite://{}", db_path.display()),
    )?
    .create_if_missing(true)
    .foreign_keys(true)
    .journal_mode(SqliteJournalMode::Wal)
    .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePool::connect_with(opts).await?;

    // Run embedded migrations.  sqlx resolves the path relative to
    // CARGO_MANIFEST_DIR (the crate root), where `migrations/` lives.
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

// ── Row types ────────────────────────────────────────────────────────────────

/// A row from the `users` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbUser {
    pub id: String,
    pub provider: String,
    pub provider_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub tier: String,
    pub created_at: String,
}

/// A row from the `sessions` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbSession {
    pub token: String,
    pub user_id: String,
    pub expires_at: String,
    pub kind: String,
}

// ── Users ────────────────────────────────────────────────────────────────────

/// Insert or update a user identified by `(provider, provider_id)`.
///
/// If the user does not exist yet, a new row is created with a fresh nanoid
/// and `tier = 'free'`.  On conflict the `email` and `display_name` columns
/// are refreshed, then the full current row is returned.
pub async fn upsert_user(
    pool: &SqlitePool,
    provider: &str,
    provider_id: &str,
    email: Option<&str>,
    display_name: Option<&str>,
) -> Result<DbUser, sqlx::Error> {
    let new_id = nanoid::nanoid!();
    let now = Utc::now().to_rfc3339();

    // Insert the row if it does not already exist; otherwise do nothing so
    // that tier and created_at are preserved.
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO users (id, provider, provider_id, email, display_name, tier, created_at)
        VALUES (?, ?, ?, ?, ?, 'free', ?)
        "#,
    )
    .bind(&new_id)
    .bind(provider)
    .bind(provider_id)
    .bind(email)
    .bind(display_name)
    .bind(&now)
    .execute(pool)
    .await?;

    // Always refresh mutable profile fields in case they changed upstream.
    sqlx::query(
        r#"
        UPDATE users SET email = ?, display_name = ?
        WHERE provider = ? AND provider_id = ?
        "#,
    )
    .bind(email)
    .bind(display_name)
    .bind(provider)
    .bind(provider_id)
    .execute(pool)
    .await?;

    // Fetch and return the canonical row.
    let user = sqlx::query_as::<_, DbUser>(
        r#"
        SELECT id, provider, provider_id, email, display_name, tier, created_at
        FROM users
        WHERE provider = ? AND provider_id = ?
        "#,
    )
    .bind(provider)
    .bind(provider_id)
    .fetch_one(pool)
    .await?;

    Ok(user)
}

// ── Sessions ─────────────────────────────────────────────────────────────────

/// Create a new session for `user_id` with the given `kind` and TTL.
pub async fn create_session(
    pool: &SqlitePool,
    user_id: &str,
    kind: &str,
    ttl_days: u32,
) -> Result<DbSession, sqlx::Error> {
    let token = nanoid::nanoid!(48);
    let expires_at = (Utc::now() + chrono::Duration::days(ttl_days as i64)).to_rfc3339();

    sqlx::query(
        r#"
        INSERT INTO sessions (token, user_id, expires_at, kind)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(&token)
    .bind(user_id)
    .bind(&expires_at)
    .bind(kind)
    .execute(pool)
    .await?;

    Ok(DbSession {
        token,
        user_id: user_id.to_owned(),
        expires_at,
        kind: kind.to_owned(),
    })
}

/// Look up a non-expired session by `token` and return the owning user, if any.
pub async fn get_session(
    pool: &SqlitePool,
    token: &str,
) -> Result<Option<DbUser>, sqlx::Error> {
    let user = sqlx::query_as::<_, DbUser>(
        r#"
        SELECT u.id, u.provider, u.provider_id, u.email, u.display_name, u.tier, u.created_at
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token = ?
          AND s.expires_at > datetime('now')
        "#,
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Delete the session identified by `token`.
pub async fn delete_session(pool: &SqlitePool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete all sessions belonging to `user_id` (e.g. on sign-out-everywhere).
pub async fn delete_all_sessions(pool: &SqlitePool, user_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Project ownership ────────────────────────────────────────────────────────

/// Record that `user_id` owns `project_id`.  Silently ignores duplicates.
pub async fn set_project_owner(
    pool: &SqlitePool,
    project_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO project_owners (project_id, user_id) VALUES (?, ?)",
    )
    .bind(project_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return the owner's user ID for `project_id`, or `None` if unclaimed.
pub async fn get_project_owner(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM project_owners WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(uid,)| uid))
}

/// Count how many projects from `project_ids` (the full set known to the FS
/// store) are owned by `user_id`.
///
/// Returns `0` immediately when `project_ids` is empty.
pub async fn count_live_projects(
    pool: &SqlitePool,
    user_id: &str,
    project_ids: &[String],
) -> Result<usize, sqlx::Error> {
    if project_ids.is_empty() {
        return Ok(0);
    }

    // Build `SELECT COUNT(*) FROM project_owners WHERE user_id = ? AND project_id IN (?,?,…)`
    // dynamically because sqlx does not support slice binding for IN clauses.
    let placeholders = project_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT COUNT(*) FROM project_owners WHERE user_id = ? AND project_id IN ({placeholders})"
    );

    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(user_id);
    for pid in project_ids {
        q = q.bind(pid);
    }

    let count: i64 = q.fetch_one(pool).await?;
    Ok(count as usize)
}
