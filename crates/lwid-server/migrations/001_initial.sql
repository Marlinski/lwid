-- 001_initial.sql: auth schema

CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    provider     TEXT NOT NULL,
    provider_id  TEXT NOT NULL,
    email        TEXT,
    display_name TEXT,
    tier         TEXT NOT NULL DEFAULT 'free',
    created_at   TEXT NOT NULL,
    UNIQUE(provider, provider_id)
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    kind       TEXT NOT NULL DEFAULT 'browser'
);

CREATE TABLE IF NOT EXISTS project_owners (
    project_id TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE
);
