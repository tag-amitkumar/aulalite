-- migrations/20260517000020_app_role.sql
--
-- Creates `aulalite_app`, the non-superuser role the backend should connect
-- as in production so that PostgreSQL row-level security policies actually
-- enforce per-tenant isolation. Superusers (and users with the BYPASSRLS
-- attribute) ignore RLS regardless of `FORCE ROW LEVEL SECURITY`, so the
-- existing `aulalite` connection role silently disables the policies in
-- earlier migrations (`force_rls.sql`, `courses.sql`, etc.).
--
-- Operational steps to actually switch in production:
--   1. Run this migration.
--   2. ALTER USER aulalite_app WITH PASSWORD '<set in your secret store>';
--   3. Change DATABASE_URL on the backend to use `aulalite_app:<password>`.
--   4. Confirm all handlers that touch tenant-scoped tables call
--      `SELECT set_config('app.tenant_id', $1, true)` at the start of
--      their transaction (or use the helper added in `db::mod`). Without
--      this the policies will filter out all rows.
--
-- The migration is idempotent: re-running it on an environment that
-- already has the role is a no-op.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aulalite_app') THEN
        CREATE ROLE aulalite_app LOGIN
            NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE NOREPLICATION
            PASSWORD NULL;
    END IF;
END$$;

-- Grants. Use NOT EXISTS so re-runs are idempotent.
GRANT USAGE ON SCHEMA public TO aulalite_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO aulalite_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO aulalite_app;

-- Default privileges so newly-created tables/sequences inherit the same
-- grants (so future migrations don't need to grant manually). These apply
-- to objects created by the current session role going forward.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO aulalite_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO aulalite_app;
