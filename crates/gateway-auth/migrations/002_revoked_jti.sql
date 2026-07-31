CREATE TABLE IF NOT EXISTS revoked_jti (
    jti TEXT PRIMARY KEY NOT NULL,
    revoked_at TEXT NOT NULL,
    exp INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_revoked_jti_exp ON revoked_jti(exp);
