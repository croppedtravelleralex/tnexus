-- Grok schema G0: media jobs/assets + egress nodes/traffic
-- Ported from Grok Go schema (relational/models.go + schema.go schemaIndexes).
-- Apply after migrations/013_grok_inference.sql.

-- =====================================================================
-- Media jobs (async video/image generation state)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_media_jobs (
    id               TEXT NOT NULL PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 64),
    request_id       TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 64),
    client_key_id    BIGINT NOT NULL CHECK (client_key_id > 0),
    client_key_name  TEXT NOT NULL DEFAULT '' CHECK (length(client_key_name) <= 160),
    account_id       BIGINT NOT NULL CHECK (account_id > 0),
    account_name     TEXT NOT NULL DEFAULT '' CHECK (length(account_name) <= 160),
    provider         TEXT NOT NULL CHECK (provider IN ('grok_web')),
    model            TEXT NOT NULL CHECK (length(trim(model)) BETWEEN 1 AND 255),
    model_route_id   BIGINT NOT NULL CHECK (model_route_id > 0),
    upstream_model   TEXT NOT NULL CHECK (length(trim(upstream_model)) BETWEEN 1 AND 255),
    prompt           TEXT NOT NULL CHECK (length(prompt) BETWEEN 0 AND 100000),
    seconds          INTEGER NOT NULL CHECK (seconds BETWEEN 1 AND 15),
    size             TEXT NOT NULL CHECK (length(trim(size)) BETWEEN 1 AND 32),
    quality          TEXT NOT NULL CHECK (length(trim(quality)) BETWEEN 1 AND 32),
    status           TEXT NOT NULL CHECK (status IN ('queued','in_progress','completed','failed')),
    progress         INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    input_json       TEXT NOT NULL DEFAULT '{}' CHECK (length(input_json) <= 1048576),
    upstream_url     TEXT NOT NULL DEFAULT '' CHECK (length(upstream_url) <= 8192),
    content_type     TEXT NOT NULL DEFAULT '' CHECK (length(content_type) <= 128),
    error_code       TEXT NOT NULL DEFAULT '' CHECK (length(error_code) <= 100),
    error_message    TEXT NOT NULL DEFAULT '' CHECK (length(error_message) <= 512),
    lease_until      TIMESTAMPTZ,
    claim_token      TEXT NOT NULL DEFAULT '' CHECK (claim_token = '' OR length(claim_token) BETWEEN 16 AND 64),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at     TIMESTAMPTZ,
    usage_recorded_at TIMESTAMPTZ,
    CONSTRAINT fk_grok_media_jobs_account FOREIGN KEY (account_id) REFERENCES grok_accounts(id) ON DELETE RESTRICT,
    CONSTRAINT fk_grok_media_jobs_client_key FOREIGN KEY (client_key_id) REFERENCES grok_client_keys(id) ON DELETE RESTRICT
);

-- =====================================================================
-- Media assets (R2/object archive)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_media_assets (
    id                     TEXT NOT NULL PRIMARY KEY CHECK (length(trim(id)) BETWEEN 16 AND 64),
    kind                   TEXT NOT NULL CHECK (kind IN ('image')),
    storage_key            TEXT NOT NULL CHECK (length(trim(storage_key)) BETWEEN 1 AND 512),
    mime_type              TEXT NOT NULL CHECK (mime_type IN ('image/jpeg','image/png','image/webp','image/gif')),
    size_bytes             BIGINT NOT NULL CHECK (size_bytes > 0 AND size_bytes <= 33554432),
    sha256                 TEXT NOT NULL CHECK (length(sha256) = 64),
    request_id             TEXT NOT NULL DEFAULT '',
    model                  TEXT NOT NULL DEFAULT '',
    resolution             TEXT NOT NULL DEFAULT '',
    width                  INTEGER NOT NULL DEFAULT 0 CHECK (width >= 0 AND width <= 100000),
    height                 INTEGER NOT NULL DEFAULT 0 CHECK (height >= 0 AND height <= 100000),
    generation_duration_ms BIGINT NOT NULL DEFAULT 0 CHECK (generation_duration_ms >= 0),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_grok_media_assets_storage_key UNIQUE (storage_key)
);

-- =====================================================================
-- Egress nodes (per-scope egress, incl. web-specific UA/CF)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_egress_nodes (
    id                        BIGSERIAL PRIMARY KEY,
    name                      TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 160),
    scope                     TEXT NOT NULL CHECK (scope IN ('grok_build','grok_web','grok_console','grok_web_asset')),
    enabled                   BOOLEAN NOT NULL DEFAULT TRUE,
    encrypted_proxy_url       TEXT NOT NULL DEFAULT '' CHECK (length(encrypted_proxy_url) <= 65536),
    user_agent                TEXT NOT NULL DEFAULT '' CHECK (length(user_agent) <= 512),
    encrypted_cloudflare_cookie TEXT NOT NULL DEFAULT '' CHECK (length(encrypted_cloudflare_cookie) <= 65536),
    health                    DOUBLE PRECISION NOT NULL DEFAULT 1 CHECK (health >= 0 AND health <= 1),
    failure_count             INTEGER NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
    cooldown_until            TIMESTAMPTZ,
    last_error                TEXT CHECK (length(last_error) <= 512),
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Egress traffic hops
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_egress_traffic_hops (
    id             BIGSERIAL PRIMARY KEY,
    request_id     TEXT NOT NULL CHECK (length(request_id) <= 128),
    egress_node_id BIGINT NOT NULL,
    egress_scope   TEXT NOT NULL CHECK (length(egress_scope) <= 32),
    provider       TEXT NOT NULL DEFAULT '' CHECK (length(provider) <= 32),
    operation      TEXT NOT NULL DEFAULT '' CHECK (length(operation) <= 64),
    pipeline_stage TEXT NOT NULL DEFAULT '' CHECK (length(pipeline_stage) <= 32),
    account_id     BIGINT NOT NULL DEFAULT 0,
    request_bytes  BIGINT NOT NULL DEFAULT 0,
    response_bytes BIGINT NOT NULL DEFAULT 0,
    transport      TEXT NOT NULL DEFAULT 'tls_client' CHECK (length(transport) <= 32),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT fk_grok_egress_traffic_node FOREIGN KEY (egress_node_id) REFERENCES grok_egress_nodes(id) ON DELETE RESTRICT
);

-- =====================================================================
-- Indexes (aligned with Go schemaIndexes)
-- =====================================================================
CREATE INDEX IF NOT EXISTS idx_grok_media_jobs_client_created
    ON grok_media_jobs (client_key_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_grok_media_jobs_recovery
    ON grok_media_jobs (status, lease_until, created_at, id);

CREATE INDEX IF NOT EXISTS idx_grok_media_jobs_usage_recovery
    ON grok_media_jobs (status, usage_recorded_at, completed_at, id);

CREATE INDEX IF NOT EXISTS idx_grok_media_assets_created
    ON grok_media_assets (created_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_grok_egress_nodes_scope_health
    ON grok_egress_nodes (scope, enabled, health DESC, id ASC);

CREATE INDEX IF NOT EXISTS idx_grok_egress_traffic_request
    ON grok_egress_traffic_hops (request_id);

CREATE INDEX IF NOT EXISTS idx_grok_egress_traffic_scope_created
    ON grok_egress_traffic_hops (egress_scope, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_grok_egress_traffic_node_created
    ON grok_egress_traffic_hops (egress_node_id, created_at DESC);
