-- B2: gateway 可展示 URL，不经 R2 存图
ALTER TABLE job_results
    ADD COLUMN IF NOT EXISTS source_url TEXT;
