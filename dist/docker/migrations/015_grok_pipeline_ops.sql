-- Grok schema G0: image pipeline traces/segments + chrome tickets + runtime settings
-- Ported from Grok Go schema (relational/image_pipeline_models.go, chrome_ticket_models.go,
-- models.go runtimeSettingsModel + schema.go schemaIndexes).
-- Apply after migrations/014_grok_media_egress.sql.

-- =====================================================================
-- Image pipeline traces (v2 phased generation)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_pipeline_traces (
    id                TEXT NOT NULL PRIMARY KEY CHECK (length(trim(id)) BETWEEN 16 AND 64),
    request_id        TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 64),
    lane              INTEGER NOT NULL DEFAULT -1 CHECK (lane >= -1 AND lane <= 128),
    status            TEXT NOT NULL CHECK (status IN ('queued','running','succeeded','failed','canceled')),
    model             TEXT NOT NULL DEFAULT '' CHECK (length(model) <= 255),
    account_id        BIGINT,
    account_name      TEXT NOT NULL DEFAULT '' CHECK (length(account_name) <= 160),
    error_code        TEXT NOT NULL DEFAULT '' CHECK (length(error_code) <= 100),
    started_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at          TIMESTAMPTZ,
    queue_ms          BIGINT NOT NULL DEFAULT 0,
    upload_queue_ms   BIGINT NOT NULL DEFAULT 0,
    ps_queue_ms       BIGINT NOT NULL DEFAULT 0,
    ss_queue_ms       BIGINT NOT NULL DEFAULT 0,
    download_queue_ms BIGINT NOT NULL DEFAULT 0,
    expand_ms         BIGINT NOT NULL DEFAULT 0,
    ssems             BIGINT NOT NULL DEFAULT 0,
    download_ms       BIGINT NOT NULL DEFAULT 0,
    total_ms          BIGINT NOT NULL DEFAULT 0,
    soft_stop         BOOLEAN NOT NULL DEFAULT FALSE
);

-- =====================================================================
-- Image pipeline segments (per-stage timing)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_pipeline_segments (
    id         BIGSERIAL PRIMARY KEY,
    trace_id   TEXT NOT NULL CHECK (length(trim(trace_id)) BETWEEN 16 AND 64),
    stage      TEXT NOT NULL CHECK (stage IN ('queue','upload','queue_upload','queue_ps','ps','expand','queue_ss','sse','queue_download','download')),
    slot       INTEGER NOT NULL DEFAULT -1 CHECK (slot >= -1 AND slot <= 128),
    sequence   INTEGER NOT NULL CHECK (sequence >= 0 AND sequence <= 1000),
    started_at TIMESTAMPTZ NOT NULL,
    ended_at   TIMESTAMPTZ,
    outcome    TEXT NOT NULL DEFAULT '' CHECK (length(outcome) <= 64),
    CONSTRAINT fk_grok_pipeline_segments_trace FOREIGN KEY (trace_id) REFERENCES grok_pipeline_traces(id) ON DELETE CASCADE
);

-- =====================================================================
-- Chrome tickets (JIT Statsig/device evidence per account)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_chrome_tickets (
    id            TEXT NOT NULL PRIMARY KEY CHECK (length(trim(id)) BETWEEN 16 AND 64),
    account_id    BIGINT NOT NULL CHECK (account_id > 0),
    statsig_meta  TEXT NOT NULL CHECK (length(trim(statsig_meta)) > 0),
    device_cookie TEXT NOT NULL DEFAULT '',
    user_agent    TEXT NOT NULL DEFAULT '' CHECK (length(user_agent) <= 512),
    sign_source   TEXT NOT NULL DEFAULT '' CHECK (length(sign_source) <= 64),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    consumed_at   TIMESTAMPTZ,
    status        TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available','consumed','expired'))
);

-- =====================================================================
-- Runtime settings (hot-reload revision)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_runtime_settings (
    key        TEXT NOT NULL PRIMARY KEY CHECK (length(trim(key)) BETWEEN 1 AND 64),
    value_json TEXT NOT NULL CHECK (length(value_json) <= 1048576),
    revision   BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Indexes (aligned with Go schemaIndexes + model index tags)
-- =====================================================================
CREATE INDEX IF NOT EXISTS idx_grok_pipeline_traces_request
    ON grok_pipeline_traces (request_id);

CREATE INDEX IF NOT EXISTS idx_grok_pipeline_traces_started
    ON grok_pipeline_traces (started_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_grok_pipeline_segments_trace
    ON grok_pipeline_segments (trace_id, sequence ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_grok_chrome_tickets_avail
    ON grok_chrome_tickets (status, expires_at, account_id, created_at);

CREATE INDEX IF NOT EXISTS idx_grok_chrome_tickets_account
    ON grok_chrome_tickets (account_id);
