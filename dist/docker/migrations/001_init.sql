CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
    display_name TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    disabled_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('director', 'casting')),
    workflow_path TEXT NOT NULL CHECK (workflow_path IN ('full_agent', 'keyword_ps')),
    ps_enabled BOOLEAN NOT NULL DEFAULT false,
    provider TEXT NOT NULL CHECK (provider IN ('chatgpt', 'grok', 'both')),
    director_factors JSONB NOT NULL DEFAULT '{}',
    ps_factors JSONB NOT NULL DEFAULT '{}',
    input_prompt TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_jobs_user_created ON jobs(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);

CREATE TABLE IF NOT EXISTS job_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('chatgpt', 'grok')),
    r2_key_original TEXT,
    r2_key_preview TEXT,
    r2_key_thumb TEXT,
    agent_prompt TEXT,
    revised_prompt TEXT,
    keywords JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_job_results_job ON job_results(job_id);
