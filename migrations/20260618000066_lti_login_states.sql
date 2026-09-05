-- Short-lived server-side state for one in-flight LTI 1.3 OIDC login.
-- Created at /v1/lti/login and atomically consumed (DELETE ... RETURNING) at
-- /v1/lti/launch to validate the CSRF state and recover the nonce. Both
-- endpoints are unauthenticated and run before tenant context is known, so the
-- table is reached exclusively through system-context policies
-- (app.system='on'). The 15-minute TTL is enforced by db::lti::take_login_state
-- with opportunistic cleanup of expired rows.

CREATE TABLE lti_login_states (
    state TEXT PRIMARY KEY,
    nonce TEXT NOT NULL,
    target_link_uri TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX lti_login_states_created_idx ON lti_login_states (created_at);

ALTER TABLE lti_login_states ENABLE ROW LEVEL SECURITY;
ALTER TABLE lti_login_states FORCE ROW LEVEL SECURITY;

CREATE POLICY system_context_select ON lti_login_states
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_insert ON lti_login_states
    FOR INSERT
    WITH CHECK (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_delete ON lti_login_states
    FOR DELETE
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON lti_login_states TO aulalite_app;

