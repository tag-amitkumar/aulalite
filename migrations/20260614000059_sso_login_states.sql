-- Short-lived server-side state for one in-flight OIDC authorization-code
-- login. Created at /v1/sso/:slug/start and atomically consumed (DELETE…
-- RETURNING) at /v1/sso/callback to validate the CSRF `state` and recover the
-- `nonce`. Both endpoints are UNAUTHENTICATED and run with no app.tenant_id, so
-- the table is reached EXCLUSIVELY via the system_context policies
-- (app.system='on'); there is no per-tenant access path. 15-minute TTL is
-- enforced in SQL by db::sso::take_login_state (+ opportunistic GC).

CREATE TABLE sso_login_states (
    state TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    nonce TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX sso_login_states_created_idx ON sso_login_states (created_at);

ALTER TABLE sso_login_states ENABLE ROW LEVEL SECURITY;
ALTER TABLE sso_login_states FORCE ROW LEVEL SECURITY;
-- Insert + select + delete only under the trusted system context.
CREATE POLICY system_context_select ON sso_login_states
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_insert ON sso_login_states
    FOR INSERT
    WITH CHECK (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_delete ON sso_login_states
    FOR DELETE
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON sso_login_states TO aulalite_app;
