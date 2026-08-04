-- Grok schema G0: tier/quota/model state families
-- Ported from Grok Go schema (relational/models.go + schema.go schemaIndexes).
-- Apply after migrations/010_grok_core.sql.

-- =====================================================================
-- Quota windows (fast/auto/imagine per account.mode)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_quota_windows (
    account_id     BIGINT  NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    mode           TEXT    NOT NULL CHECK (length(trim(mode)) BETWEEN 1 AND 64),
    remaining      INTEGER NOT NULL DEFAULT 0 CHECK (remaining >= 0),
    total          INTEGER NOT NULL DEFAULT 0 CHECK (total >= 0),
    usage_percent  DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (usage_percent >= 0 AND usage_percent <= 100),
    breakdown_json TEXT    NOT NULL DEFAULT '[]' CHECK (length(breakdown_json) <= 8192),
    window_seconds INTEGER NOT NULL DEFAULT 0 CHECK (window_seconds >= 0),
    reset_at       TIMESTAMPTZ,
    synced_at      TIMESTAMPTZ,
    source         TEXT    NOT NULL CHECK (source IN ('default','estimated','upstream')),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, mode)
);

-- =====================================================================
-- Billing snapshots (Build paid plans)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_billing_snapshots (
    account_id                BIGINT NOT NULL PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    plan_code                 TEXT   CHECK (length(plan_code) <= 100),
    plan_name                 TEXT   CHECK (length(plan_name) <= 160),
    monthly_limit             DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (monthly_limit >= 0),
    used                      DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (used >= 0),
    on_demand_cap             DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (on_demand_cap >= 0),
    on_demand_used            DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (on_demand_used >= 0),
    prepaid_balance           DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (prepaid_balance >= 0),
    credit_usage_percent      DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (credit_usage_percent >= 0),
    is_unified_billing_user   BOOLEAN NOT NULL DEFAULT FALSE,
    top_up_method             TEXT   CHECK (length(top_up_method) <= 100),
    usage_period_type         TEXT   CHECK (length(usage_period_type) <= 100),
    usage_period_start        TEXT   CHECK (length(usage_period_start) <= 64),
    usage_period_end          TEXT   CHECK (length(usage_period_end) <= 64),
    billing_period_start      TEXT   CHECK (length(billing_period_start) <= 64),
    billing_period_end        TEXT   CHECK (length(billing_period_end) <= 64),
    history_json              TEXT   NOT NULL DEFAULT '[]' CHECK (length(history_json) <= 1048576),
    synced_at                 TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Pool snapshots (15-min analytics per provider)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_pool_snapshots (
    id              BIGSERIAL PRIMARY KEY,
    bucket_at       TIMESTAMPTZ NOT NULL,
    provider        TEXT NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    total           BIGINT NOT NULL DEFAULT 0,
    available       BIGINT NOT NULL DEFAULT 0,
    cooldown        BIGINT NOT NULL DEFAULT 0,
    waiting_reset   BIGINT NOT NULL DEFAULT 0,
    probing         BIGINT NOT NULL DEFAULT 0,
    disabled        BIGINT NOT NULL DEFAULT 0,
    reauth_required BIGINT NOT NULL DEFAULT 0,
    free            BIGINT NOT NULL DEFAULT 0,
    paid            BIGINT NOT NULL DEFAULT 0,
    unknown         BIGINT NOT NULL DEFAULT 0,
    tier_auto       BIGINT NOT NULL DEFAULT 0,
    tier_basic      BIGINT NOT NULL DEFAULT 0,
    tier_super      BIGINT NOT NULL DEFAULT 0,
    tier_heavy      BIGINT NOT NULL DEFAULT 0,
    quota_remaining DOUBLE PRECISION NOT NULL DEFAULT 0,
    quota_total     DOUBLE PRECISION NOT NULL DEFAULT 0,
    quota_known     BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Quota recovery queue (DB portion)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_quota_recovery (
    account_id        BIGINT NOT NULL PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('free','paid')),
    status            TEXT NOT NULL CHECK (status IN ('exhausted','probing')),
    confirmed_used    BIGINT NOT NULL DEFAULT 0 CHECK (confirmed_used >= 0),
    confirmed_limit   BIGINT NOT NULL DEFAULT 0 CHECK (confirmed_limit >= 0),
    exhausted_at      TIMESTAMPTZ,
    next_probe_at     TIMESTAMPTZ,
    last_confirmed_at TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- =====================================================================
-- Account model capability
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_capabilities (
    account_id     BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    upstream_model TEXT NOT NULL CHECK (length(trim(upstream_model)) BETWEEN 1 AND 255),
    PRIMARY KEY (account_id, upstream_model)
);

-- =====================================================================
-- Account model sync state (accountsync)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_sync_states (
    account_id      BIGINT NOT NULL PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    last_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_success_at TIMESTAMPTZ,
    last_error      TEXT CHECK (length(last_error) <= 512)
);

-- =====================================================================
-- Account model quota blocks (cooldown blocks)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_quota_blocks (
    account_id      BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    upstream_model  TEXT NOT NULL CHECK (length(trim(upstream_model)) BETWEEN 1 AND 255),
    reason          TEXT NOT NULL CHECK (length(trim(reason)) BETWEEN 1 AND 100),
    cooldown_until  TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, upstream_model)
);

-- =====================================================================
-- Account model states (probe result)
-- =====================================================================
CREATE TABLE IF NOT EXISTS grok_model_states (
    account_id            BIGINT NOT NULL REFERENCES grok_accounts(id) ON DELETE CASCADE,
    upstream_model        TEXT NOT NULL CHECK (length(trim(upstream_model)) BETWEEN 1 AND 255),
    status                TEXT NOT NULL CHECK (status IN ('unknown','quota_available','available','soft_stop','quota_exhausted','auth_failed','signature_failed')),
    reason                TEXT NOT NULL DEFAULT '' CHECK (length(reason) <= 100),
    consecutive_failures  INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0),
    last_attempt_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_success_at       TIMESTAMPTZ,
    cooldown_until        TIMESTAMPTZ,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, upstream_model)
);

-- =====================================================================
-- Indexes (aligned with Go schemaIndexes)
-- =====================================================================
CREATE INDEX IF NOT EXISTS idx_grok_quota_windows_due
    ON grok_quota_windows (remaining, reset_at, account_id);

CREATE INDEX IF NOT EXISTS idx_grok_quota_recovery_status_probe
    ON grok_quota_recovery (status, next_probe_at, account_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_grok_pool_snapshots_bucket_provider
    ON grok_pool_snapshots (bucket_at, provider);

CREATE INDEX IF NOT EXISTS idx_grok_pool_snapshots_bucket
    ON grok_pool_snapshots (bucket_at DESC, provider);

CREATE INDEX IF NOT EXISTS idx_grok_model_quota_blocks_due
    ON grok_model_quota_blocks (cooldown_until, account_id);

CREATE INDEX IF NOT EXISTS idx_grok_model_states_status
    ON grok_model_states (upstream_model, status, cooldown_until, account_id);
