-- 图片二进制存 R2；DB 仅存元数据。无 R2 时临时存缩略图 base64 便于本地预览。
ALTER TABLE job_results
    ADD COLUMN IF NOT EXISTS inline_preview_b64 TEXT;

CREATE INDEX IF NOT EXISTS idx_jobs_user_status_created
    ON jobs(user_id, status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_job_results_provider
    ON job_results(job_id, provider);
