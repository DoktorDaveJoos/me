CREATE TABLE IF NOT EXISTS me_account (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    auth_hash TEXT NOT NULL,
    recovery_hash TEXT NOT NULL,
    vault_id TEXT NOT NULL UNIQUE,
    envelope JSONB NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
