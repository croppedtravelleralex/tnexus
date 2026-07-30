-- 构思模型列表（文本 LLM，仅负责 prompt）
ALTER TABLE jobs
    ADD COLUMN IF NOT EXISTS director_models JSONB NOT NULL DEFAULT '["gpt"]'::jsonb;

-- 结果 provider 改为自由文本（构思模型 id，如 gpt / deepseek）
ALTER TABLE job_results DROP CONSTRAINT IF EXISTS job_results_provider_check;
