-- migrations/20260530000030_system_context_rls.sql
--
-- Lets the trusted in-process background workers (recording sweep, retention
-- janitor, auto-end sweep) read/act ACROSS tenants when the backend connects as
-- the non-superuser `aulalite_app` role (migration 20260517000020).
--
-- The problem this fixes: those workers run discovery queries on a connection
-- that has no `app.tenant_id` set, so the per-table `tenant_isolation` policies
-- (`USING tenant_id = current_setting('app.tenant_id', true)`) evaluate to NULL
-- and silently filter out EVERY row. Under a superuser/BYPASSRLS role it happens
-- to work (RLS is ignored); under `aulalite_app` the sweeps become no-ops —
-- sessions never auto-end, recordings never get produced or pruned.
--
-- Mechanism: a row gains cross-tenant visibility ONLY when the transaction sets
-- the GUC `app.system = 'on'` (see `db::begin_system_context`), which exclusively
-- the trusted sweep code does. Request handlers never set it, so per-tenant
-- isolation is completely unchanged for normal traffic. These policies are
-- PERMISSIVE, so they are OR'd with `tenant_isolation` and only ADD access when
-- the system GUC is on. They are scoped to the minimum commands each sweep needs.

-- recordings: sweeps only SELECT for discovery; all writes still flow through a
-- tenant-scoped transaction (app.tenant_id), so SELECT-only here.
CREATE POLICY system_context_select ON recordings
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');

-- live_sessions: discovery SELECT + the auto-end sweep UPDATE.
CREATE POLICY system_context_select ON live_sessions
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_update ON live_sessions
    FOR UPDATE
    USING (current_setting('app.system', true) = 'on')
    WITH CHECK (current_setting('app.system', true) = 'on');

-- tenants: the retention janitor JOINs tenants to read recording_retention_days.
CREATE POLICY system_context_select ON tenants
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
