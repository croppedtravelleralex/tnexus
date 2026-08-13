-- Job result: Grok pipeline stage JSON (39c §6 W-3).
-- Captures upstream PS/SS stage timings returned under `_tnexus_pipeline`
-- so job_results carry the stage breakdown. Non-grok rows are NULL.

ALTER TABLE job_results
    ADD COLUMN IF NOT EXISTS pipeline JSONB;
