ALTER TABLE jobs
    ADD COLUMN IF NOT EXISTS gen_config JSONB NOT NULL DEFAULT '{"quality":"auto","width":1024,"height":1024,"count":1,"transparent_bg":false}'::jsonb;

ALTER TABLE job_results
    ADD COLUMN IF NOT EXISTS variant_index INT NOT NULL DEFAULT 0;
