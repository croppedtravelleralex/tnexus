-- TNexus-owned account pool (independent from gptimage accounts.db).
-- Phase P2-D: migrate from shared sqlite via one-shot ETL + dual-write window.

CREATE TABLE IF NOT EXISTS tnexus_accounts (
    id BIGSERIAL PRIMARY KEY,
    email TEXT NOT NULL,
    access_token TEXT NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT tnexus_accounts_email_unique UNIQUE (email),
    CONSTRAINT tnexus_accounts_token_unique UNIQUE (access_token)
);

CREATE INDEX IF NOT EXISTS idx_tnexus_accounts_email ON tnexus_accounts (email);
CREATE INDEX IF NOT EXISTS idx_tnexus_accounts_updated ON tnexus_accounts (updated_at DESC);

-- Mirror scheduling / inflight fields for gateway without live sqlite reads (future).
CREATE TABLE IF NOT EXISTS tnexus_account_runtime (
    email TEXT PRIMARY KEY REFERENCES tnexus_accounts (email) ON DELETE CASCADE,
    scheduling_state TEXT NOT NULL DEFAULT 'verified_ready',
    image_inflight INTEGER NOT NULL DEFAULT 0,
    quota INTEGER,
    image_quota_unknown BOOLEAN NOT NULL DEFAULT false,
    soft_band_percent INTEGER,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
