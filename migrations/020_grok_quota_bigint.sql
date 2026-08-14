-- Grok quota remaining/total：生产曾因 ETL/Go 写入 NUMERIC，而
-- 019 的 ALTER TYPE 用 EXCEPTION WHEN duplicate_column 接不住 datatype 失败。
-- 幂等扩为 BIGINT，USING 兼容 INTEGER/NUMERIC。

DO $$
BEGIN
    ALTER TABLE grok_quota_windows
        ALTER COLUMN remaining TYPE BIGINT USING remaining::bigint;
    ALTER TABLE grok_quota_windows
        ALTER COLUMN total TYPE BIGINT USING total::bigint;
EXCEPTION WHEN OTHERS THEN
    NULL;
END $$;
