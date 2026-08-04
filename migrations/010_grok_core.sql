-- Grok schema G0: core accounts (full parity with Go schema.go / models.go)
-- Apply after migrations/009_tnexus_accounts.sql
-- Source of truth:
--   grokImage/backend/internal/infra/persistence/relational/{schema.go,models.go}
--   docs/39b-grok-schema.md §3 + §3.1
--
-- NOTE: column names / CHECKs mirror the Go GORM models verbatim so the ETL
-- (scripts/grok_etl_sqlite_to_pg.py) column-intersection COPY preserves
-- ciphertext (account_credentials.encrypted_primary / encrypted_refresh).

-- administratable ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_admins (
    id            BIGSERIAL PRIMARY KEY,
    username      VARCHAR(100) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Go: username uniqueIndex + check chk_admins_username
CREATE UNIQUE INDEX IF NOT EXISTS idx_grok_admins_username ON grok_admins (username);

-- admin sessions ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_admin_sessions (
    id                BIGSERIAL PRIMARY KEY,
    admin_id          BIGINT NOT NULL REFERENCES grok_admins(id) ON DELETE CASCADE,
    refresh_token_hash VARCHAR(64) NOT NULL,
    expires_at        TIMESTAMPTZ NOT NULL,
    last_used_at      TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Go: refresh_token_hash uniqueIndex (data-integrity constraint)
CREATE UNIQUE INDEX IF NOT EXISTS idx_grok_admin_sessions_token_hash ON grok_admin_sessions (refresh_token_hash);
-- Go schemaIndexes
CREATE INDEX IF NOT EXISTS idx_grok_admin_sessions_admin_created
    ON grok_admin_sessions (admin_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_grok_admin_sessions_expires
    ON grok_admin_sessions (expires_at);

-- provider_accounts ------------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_accounts (
    id                BIGSERIAL PRIMARY KEY,
    identity_key      VARCHAR(64) NOT NULL,
    provider          VARCHAR(32) NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    name              VARCHAR(160) NOT NULL,
    email             VARCHAR(255),
    user_id           VARCHAR(255),
    team_id           VARCHAR(255),
    source_key        VARCHAR(512) NOT NULL,
    enabled           BOOLEAN NOT NULL DEFAULT true,
    auth_status       VARCHAR(32) NOT NULL DEFAULT 'active' CHECK (auth_status IN ('active','reauthRequired')),
    priority          INTEGER NOT NULL DEFAULT 1,
    max_concurrent    INTEGER NOT NULL DEFAULT 8 CHECK (max_concurrent BETWEEN 1 AND 256),
    minimum_remaining NUMERIC NOT NULL DEFAULT 0 CHECK (minimum_remaining >= 0),
    failure_count     INTEGER NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
    cooldown_until    TIMESTAMPTZ,
    last_error        VARCHAR(512),
    last_used_at      TIMESTAMPTZ,
    observed_model    VARCHAR(255),
    observed_model_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Go: provider_accounts.identity_key uniqueIndex; schema.go restores it as a
-- named unique index (idx_provider_accounts_identity_key) after SQLite rebuilds.
CREATE UNIQUE INDEX IF NOT EXISTS idx_grok_accounts_identity_key ON grok_accounts (identity_key);
-- Go schemaIndexes
CREATE INDEX IF NOT EXISTS idx_grok_accounts_routing
    ON grok_accounts (provider, enabled, auth_status, priority DESC, id ASC);
CREATE INDEX IF NOT EXISTS idx_grok_accounts_pool_observed
    ON grok_accounts (provider, enabled, auth_status, observed_model);
CREATE INDEX IF NOT EXISTS idx_grok_accounts_created_id
    ON grok_accounts (created_at DESC, id DESC);

-- account_credentials ----------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_credentials (
    account_id         BIGINT PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    auth_type          VARCHAR(16) NOT NULL CHECK (auth_type IN ('oauth','sso')),
    client_id          VARCHAR(255),
    encrypted_primary  TEXT   NOT NULL DEFAULT '',
    encrypted_refresh  TEXT   NOT NULL DEFAULT '',
    expires_at         TIMESTAMPTZ,
    refresh_due_at     TIMESTAMPTZ,
    last_refresh_at    TIMESTAMPTZ,
    refresh_failures   INTEGER NOT NULL DEFAULT 0 CHECK (refresh_failures >= 0),
    last_refresh_error VARCHAR(100) NOT NULL DEFAULT '',
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Go: check chk_account_credentials_secret (oauth/sso conditional on empties)
ALTER TABLE grok_credentials
    ADD CONSTRAINT chk_grok_credentials_secret CHECK (
        (auth_type = 'oauth'  AND (encrypted_primary <> '' OR encrypted_refresh <> ''))
        OR (auth_type = 'sso' AND encrypted_primary <> '' AND encrypted_refresh = '')
    );
-- Go schemaIndexes
CREATE INDEX IF NOT EXISTS idx_grok_credentials_refresh_due
    ON grok_credentials (refresh_due_at, account_id);

-- account_provider_links -------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_account_provider_links (
    web_account_id   BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    build_account_id BIGINT NOT NULL UNIQUE REFERENCES grok_accounts(id) ON DELETE CASCADE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (web_account_id)
);
-- Go: chk_account_provider_links_distinct (web <> build)
ALTER TABLE grok_account_provider_links
    ADD CONSTRAINT chk_grok_account_provider_links_distinct
    CHECK (web_account_id <> build_account_id);

-- web_account_profiles ---------------------------------------------------------
CREATE TABLE IF NOT EXISTS grok_web_profiles (
    account_id BIGINT PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    tier       VARCHAR(16) NOT NULL CHECK (tier IN ('auto','basic','super','heavy')),
    synced_at  TIMESTAMPTZ
);
