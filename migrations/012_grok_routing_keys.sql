-- Grok schema G0: model routes / aliases / client keys / reservations
-- Ported from Grok Go schema (relational/models.go + schema.go schemaIndexes).
-- Apply after migrations/011_grok_quota_models.sql.

-- =====================================================================
-- Model routes (对外模型路由)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_routes (
    id              BIGSERIAL PRIMARY KEY,
    public_id       TEXT NOT NULL CHECK (length(trim(public_id)) BETWEEN 1 AND 255),
    provider        TEXT NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    upstream_model  TEXT NOT NULL CHECK (length(trim(upstream_model)) BETWEEN 1 AND 255),
    capability      TEXT NOT NULL CHECK (capability IN ('responses','chat','image','image_edit','video')),
    origin          TEXT NOT NULL DEFAULT 'discovered' CHECK (origin IN ('catalog','discovered','manual')),
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- unique: public_id + (provider, upstream_model)
    CONSTRAINT uq_grok_model_routes_public_id UNIQUE (public_id),
    CONSTRAINT uq_grok_model_routes_provider_upstream UNIQUE (provider, upstream_model)
);

-- =====================================================================
-- Model route aliases (升级/重命名前兼容名，含 grok-vision-ocr)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_route_aliases (
    alias          TEXT NOT NULL PRIMARY KEY CHECK (length(trim(alias)) BETWEEN 1 AND 255),
    model_route_id BIGINT NOT NULL REFERENCES grok_model_routes(id) ON DELETE CASCADE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Model route accounts (pin 绑定)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_route_accounts (
    model_route_id BIGINT NOT NULL REFERENCES grok_model_routes(id) ON DELETE CASCADE,
    account_id     BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    PRIMARY KEY (model_route_id, account_id)
);

-- =====================================================================
-- Client keys (g2a_*)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_client_keys (
    id                      BIGSERIAL PRIMARY KEY,
    name                    TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 160),
    prefix                  TEXT NOT NULL CHECK (length(prefix) BETWEEN 1 AND 32),
    secret_hash             TEXT NOT NULL CHECK (length(secret_hash) = 64),
    encrypted_secret        TEXT NOT NULL CHECK (length(trim(encrypted_secret)) BETWEEN 1 AND 4096),
    enabled                 BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at              TIMESTAMPTZ,
    rpm_limit               INTEGER NOT NULL DEFAULT 120 CHECK (rpm_limit BETWEEN 1 AND 100000),
    max_concurrent          INTEGER NOT NULL DEFAULT 8 CHECK (max_concurrent BETWEEN 1 AND 1024),
    billing_limit_usd_ticks BIGINT NOT NULL DEFAULT 0 CHECK (billing_limit_usd_ticks BETWEEN 0 AND 9000000000000000),
    billed_usage_usd_ticks  BIGINT NOT NULL DEFAULT 0 CHECK (billed_usage_usd_ticks >= 0),
    reserved_usage_usd_ticks BIGINT NOT NULL DEFAULT 0 CHECK (reserved_usage_usd_ticks >= 0),
    last_used_at            TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_grok_client_keys_prefix UNIQUE (prefix)
);

-- =====================================================================
-- Client key model permissions (join client_key_models)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_client_key_models (
    client_key_id  BIGINT NOT NULL REFERENCES grok_client_keys(id) ON DELETE CASCADE,
    model_route_id BIGINT NOT NULL REFERENCES grok_model_routes(id) ON DELETE CASCADE,
    PRIMARY KEY (client_key_id, model_route_id)
);

-- =====================================================================
-- Billing reservations
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_billing_reservations (
    event_id      TEXT NOT NULL PRIMARY KEY CHECK (length(event_id) BETWEEN 16 AND 64),
    client_key_id BIGINT NOT NULL REFERENCES grok_client_keys(id) ON DELETE CASCADE CHECK (client_key_id > 0),
    amount        BIGINT NOT NULL CHECK (amount > 0),
    expires_at    TIMESTAMPTZ NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- grok-vision-ocr seed alias -> fast web chat route (39b §3.3)
-- Only inserted if a matching web fast route exists (populated by ETL).
-- =====================================================================
INSERT INTO grok_model_route_aliases (alias, model_route_id, created_at)
SELECT 'grok-vision-ocr', id, now()
FROM grok_model_routes
WHERE provider = 'grok_web' AND upstream_model = 'grok-chat-fast' AND enabled = TRUE
ON CONFLICT (alias) DO NOTHING;

-- =====================================================================
-- Indexes (aligned with Go schemaIndexes)
-- =====================================================================
CREATE INDEX IF NOT EXISTS idx_grok_model_routes_created_id
    ON grok_model_routes (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_model_routes_enabled
    ON grok_model_routes (enabled, public_id, id);

CREATE INDEX IF NOT EXISTS idx_grok_model_route_aliases_route
    ON grok_model_route_aliases (model_route_id, alias);

CREATE INDEX IF NOT EXISTS idx_grok_model_route_accounts_account_route
    ON grok_model_route_accounts (account_id, model_route_id);

CREATE INDEX IF NOT EXISTS idx_grok_client_keys_created_id
    ON grok_client_keys (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_client_keys_status
    ON grok_client_keys (enabled, expires_at, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_client_key_models_route_key
    ON grok_client_key_models (model_route_id, client_key_id);

CREATE INDEX IF NOT EXISTS idx_grok_billing_reservations_expiry
    ON grok_billing_reservations (expires_at, client_key_id);
