-- =============================================================================
-- Grok alignment 019: tier column + quota recovery status CHECK widening
-- =============================================================================
-- 背景（生产判定 B）：
--   ① grok_accounts 无 web_tier 列 → selector tier 排序退化同秩；
--   ② grok_quota_recovery.status CHECK 不含 'active' → save_quota_recovery(Active)
--      违反约束（Rust 侧 QuotaRecoveryStatus::Active 是有效中间态）。
-- 幂等：全部 IF NOT EXISTS / DO 块，可重复执行。

-- ── ① grok_accounts.web_tier（Go WebTier，selector 档位序）─────────────────
ALTER TABLE grok_accounts
    ADD COLUMN IF NOT EXISTS web_tier TEXT NOT NULL DEFAULT 'basic'
        CHECK (web_tier IN ('basic', 'super', 'heavy'));

CREATE INDEX IF NOT EXISTS idx_grok_accounts_tier
    ON grok_accounts (provider, web_tier, priority DESC, id ASC);

-- ── ② grok_quota_recovery.status CHECK 放开 active ──────────────────────────
-- 约束为无名内联 CHECK（自动命名 grok_quota_recovery_status_check）；为兼容
-- 已存在/改名的情况，动态按列名查找约束后 drop 重建（幂等）。
DO $$
DECLARE
    con_name TEXT;
BEGIN
    SELECT c.conname INTO con_name
      FROM pg_constraint c
      JOIN pg_class t ON t.oid = c.conrelid
     WHERE t.relname = 'grok_quota_recovery'
       AND c.contype = 'c'
       AND pg_get_constraintdef(c.oid) ILIKE '%status%'
       AND pg_get_constraintdef(c.oid) ILIKE '%exhausted%'
     LIMIT 1;
    IF con_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE grok_quota_recovery DROP CONSTRAINT %I', con_name);
    END IF;
END $$;

ALTER TABLE grok_quota_recovery
    ADD CONSTRAINT grok_quota_recovery_status_check
        CHECK (status IN ('active', 'exhausted', 'probing'));
