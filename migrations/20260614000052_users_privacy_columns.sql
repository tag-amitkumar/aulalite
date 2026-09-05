-- GDPR right-to-erasure + token revocation columns on the global `users` table.
-- `users` has no tenant_isolation RLS policy (it is cross-tenant by design);
-- the privacy data layer pins `id = $caller` on every write. Additive +
-- reversible: all three columns are nullable with no default. `aulalite_app`
-- already holds table-level DML on `users` via 20260517000020_app_role.sql
-- (GRANT ... ON ALL TABLES), and column additions need no extra grant.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS deleted_at         TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS anonymized_at      TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS tokens_valid_after TIMESTAMPTZ;

-- Single-row lookup support for the auth middleware's revocation check
-- (`SELECT is_platform_admin, tokens_valid_after FROM users WHERE id = $1`).
-- The PK already covers the id lookup; this partial index keeps the
-- revocation predicate cheap for the (rare) rows that actually carry a cutoff.
CREATE INDEX IF NOT EXISTS users_tokens_valid_after_idx
    ON users (id)
    WHERE tokens_valid_after IS NOT NULL;
