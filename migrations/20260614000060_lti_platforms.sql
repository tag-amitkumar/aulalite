-- LTI 1.3 platform registrations (Tool side). One+ per tenant; identified by
-- (issuer, client_id). Tenant-scoped under RLS like `announcements` for the
-- admin register/list/delete path; PLUS a system_context SELECT policy for the
-- UNAUTHENTICATED OIDC login + launch flows, which resolve the platform by
-- issuer cross-tenant under app.system='on' (db::lti::find_by_issuer), mirroring
-- 20260614000055_api_keys.sql / 20260614000047.

CREATE TABLE lti_platforms (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL,
    issuer TEXT NOT NULL,
    client_id TEXT NOT NULL,
    auth_login_url TEXT NOT NULL,
    jwks_url TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    default_course_id UUID REFERENCES courses(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX lti_platforms_tenant_idx ON lti_platforms (tenant_id);
-- find_by_issuer resolves on (issuer) or (issuer, client_id).
CREATE INDEX lti_platforms_issuer_idx ON lti_platforms (issuer);
CREATE UNIQUE INDEX lti_platforms_issuer_client_idx
    ON lti_platforms (issuer, client_id);

ALTER TABLE lti_platforms ENABLE ROW LEVEL SECURITY;
ALTER TABLE lti_platforms FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON lti_platforms
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- The OIDC login + launch resolve the platform with NO tenant GUC, elevating to
-- app.system='on' for the cross-tenant SELECT (db::lti::find_by_issuer).
CREATE POLICY system_context_select ON lti_platforms
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON lti_platforms TO aulalite_app;
