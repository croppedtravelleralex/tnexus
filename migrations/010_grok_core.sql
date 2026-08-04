-- Grok schema G0: core accounts (skeleton — expand per docs/39b-grok-schema.md)
-- Apply after migrations/009_tnexus_accounts.sql

CREATE TABLE IF NOT EXISTS grok_admins (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_admin_sessions (
    id         BIGSERIAL PRIMARY KEY,
    admin_id   BIGINT NOT NULL REFERENCES grok_admins(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_accounts (
    id             BIGSERIAL PRIMARY KEY,
    identity_key   TEXT NOT NULL UNIQUE,
    provider       TEXT NOT NULL CHECK (provider IN ('grok_build', 'grok_web', 'grok_console')),
    enabled        BOOLEAN NOT NULL DEFAULT true,
    auth_status    TEXT NOT NULL DEFAULT 'unknown',
    priority       INTEGER NOT NULL DEFAULT 0,
    observed_model TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_credentials (
    account_id               BIGINT PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    encrypted_access_token   BYTEA NOT NULL,
    encrypted_refresh_token  BYTEA,
    refresh_due_at           TIMESTAMPTZ,
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_account_provider_links (
    id              BIGSERIAL PRIMARY KEY,
    account_id      BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    linked_provider TEXT NOT NULL,
    linked_id       BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_web_profiles (
    account_id BIGINT PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    profile    JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_grok_accounts_routing
    ON grok_accounts (provider, enabled, auth_status, priority DESC, id ASC);

CREATE INDEX IF NOT EXISTS idx_grok_admin_sessions_admin_created
    ON grok_admin_sessions (admin_id, created_at DESC, id DESC);
