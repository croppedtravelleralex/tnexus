-- Grok schema G0: inference audit / ownership / web sticky state
-- Ported from Grok Go schema (relational/models.go + schema.go schemaIndexes).
-- Apply after migrations/012_grok_routing_keys.sql.

-- =====================================================================
-- Request audits
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_request_audits (
    id                        BIGSERIAL PRIMARY KEY,
    event_id                  TEXT CHECK (event_id = '' OR length(event_id) BETWEEN 16 AND 64),
    request_id                TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 64),
    client_key_id             BIGINT NOT NULL CHECK (client_key_id > 0),
    client_key_name           TEXT CHECK (length(client_key_name) <= 160),
    model_route_id            BIGINT NOT NULL CHECK (model_route_id > 0),
    model_public_id           TEXT CHECK (length(model_public_id) <= 255),
    model_upstream_model      TEXT CHECK (length(model_upstream_model) <= 255),
    provider                  TEXT NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    operation                 TEXT NOT NULL CHECK (operation IN ('responses','chat','messages','image','image_edit','video')),
    usage_source              TEXT NOT NULL CHECK (usage_source IN ('upstream','estimated','none')),
    account_id                BIGINT CHECK (account_id IS NULL OR account_id > 0),
    account_name              TEXT CHECK (length(account_name) <= 160),
    status_code               INTEGER NOT NULL CHECK (status_code BETWEEN 100 AND 599),
    streaming                 BOOLEAN NOT NULL DEFAULT FALSE,
    media_input_images        BIGINT NOT NULL DEFAULT 0,
    media_output_images       BIGINT NOT NULL DEFAULT 0,
    media_output_seconds      BIGINT NOT NULL DEFAULT 0,
    input_tokens              BIGINT NOT NULL DEFAULT 0,
    cached_input_tokens       BIGINT NOT NULL DEFAULT 0,
    output_tokens             BIGINT NOT NULL DEFAULT 0,
    reasoning_tokens          BIGINT NOT NULL DEFAULT 0,
    total_tokens              BIGINT NOT NULL DEFAULT 0,
    cost_in_usd_ticks         BIGINT NOT NULL DEFAULT 0,
    estimated_cost_in_usd_ticks BIGINT NOT NULL DEFAULT 0,
    pricing_model             TEXT CHECK (length(pricing_model) <= 100),
    pricing_version           TEXT CHECK (length(pricing_version) <= 20),
    num_sources_used          BIGINT NOT NULL DEFAULT 0,
    num_server_side_tools_used BIGINT NOT NULL DEFAULT 0,
    context_input_tokens      BIGINT NOT NULL DEFAULT 0,
    context_output_tokens     BIGINT NOT NULL DEFAULT 0,
    duration_ms               BIGINT NOT NULL DEFAULT 0,
    error_code                TEXT CHECK (length(error_code) <= 100),
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- all metrics non-negative (Go chk_request_audits_metrics)
    CONSTRAINT chk_grok_request_audits_metrics CHECK (
        media_input_images >= 0 AND media_output_images >= 0 AND media_output_seconds >= 0
        AND input_tokens >= 0 AND cached_input_tokens >= 0 AND output_tokens >= 0
        AND reasoning_tokens >= 0 AND total_tokens >= 0 AND cost_in_usd_ticks >= 0
        AND estimated_cost_in_usd_ticks >= 0 AND num_sources_used >= 0
        AND num_server_side_tools_used >= 0 AND context_input_tokens >= 0
        AND context_output_tokens >= 0 AND duration_ms >= 0
    )
);

-- =====================================================================
-- Response ownership (Build session ownership)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_response_ownership (
    response_id   TEXT NOT NULL PRIMARY KEY CHECK (length(response_id) BETWEEN 1 AND 255),
    account_id    BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    client_key_id BIGINT NOT NULL REFERENCES grok_client_keys(id) ON DELETE CASCADE,
    provider      TEXT NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    expires_at    TIMESTAMPTZ NOT NULL CHECK (expires_at > created_at),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Web response states (Web sticky conversation)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_web_response_states (
    response_id               TEXT NOT NULL PRIMARY KEY CHECK (length(response_id) BETWEEN 1 AND 255),
    account_id                BIGINT NOT NULL CHECK (account_id > 0),
    conversation_id           TEXT NOT NULL CHECK (length(trim(conversation_id)) BETWEEN 1 AND 255),
    upstream_parent_response_id TEXT NOT NULL CHECK (length(trim(upstream_parent_response_id)) BETWEEN 1 AND 255),
    response_json             TEXT NOT NULL CHECK (length(response_json) <= 16777216),
    status                    TEXT NOT NULL CHECK (status IN ('in_progress','completed','failed','cancelled')),
    expires_at                TIMESTAMPTZ NOT NULL CHECK (expires_at > created_at),
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Indexes (aligned with Go schemaIndexes)
-- =====================================================================
CREATE INDEX IF NOT EXISTS idx_grok_audits_created_id
    ON grok_request_audits (created_at DESC, id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_grok_audits_event_id
    ON grok_request_audits (event_id) WHERE event_id <> '';

CREATE INDEX IF NOT EXISTS idx_grok_audits_account_created_id
    ON grok_request_audits (account_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_audits_status_created_id
    ON grok_request_audits (status_code, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_audits_streaming_created_id
    ON grok_request_audits (streaming, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_grok_response_ownership_expires
    ON grok_response_ownership (expires_at);

CREATE INDEX IF NOT EXISTS idx_grok_response_ownership_account
    ON grok_response_ownership (account_id);

CREATE INDEX IF NOT EXISTS idx_grok_response_ownership_client_key
    ON grok_response_ownership (client_key_id);

CREATE INDEX IF NOT EXISTS idx_grok_web_response_states_expires
    ON grok_web_response_states (expires_at);

CREATE INDEX IF NOT EXISTS idx_grok_web_response_states_account
    ON grok_web_response_states (account_id, created_at DESC);
