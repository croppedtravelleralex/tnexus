-- Phase 0: OpenAPI (gateway) image archive + NewAPI user binding

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS newapi_user_id BIGINT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_newapi_user_id
    ON users(newapi_user_id)
    WHERE newapi_user_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS user_image_records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    source TEXT NOT NULL DEFAULT 'gateway_openapi'
        CHECK (source IN ('gateway_openapi', 'worker')),
    newapi_user_id BIGINT,
    newapi_token_name TEXT,
    model TEXT NOT NULL DEFAULT 'gpt-image-2',
    prompt TEXT NOT NULL DEFAULT '',
    agent_prompt TEXT,
    r2_key_original TEXT,
    r2_key_preview TEXT,
    r2_key_thumb TEXT,
    inline_preview_b64 TEXT,
    source_url TEXT,
    width INTEGER,
    height INTEGER,
    size_bytes BIGINT,
    generation_ms BIGINT,
    keywords JSONB,
    pipeline JSONB,
    usage JSONB,
    backup_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (backup_status IN ('pending', 'backed_up', 'server_purged')),
    staging_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_user_image_records_owner_created
    ON user_image_records(owner_user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_user_image_records_newapi_user
    ON user_image_records(newapi_user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_user_image_records_staging_expires
    ON user_image_records(staging_expires_at)
    WHERE staging_expires_at IS NOT NULL;
