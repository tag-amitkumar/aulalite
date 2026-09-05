-- Per-tenant OIDC IdP configuration for enterprise SSO. One row per tenant.
-- The org-admin management path is tenant-scoped under RLS exactly like
-- `announcements` (policy keys solely on app.tenant_id), enforced under the
-- non-bypass `aulalite_app` role. PLUS a system_context SELECT so the
-- UNAUTHENTICATED /v1/sso/:slug/start + /callback handlers can resolve a
-- tenant's config cross-tenant before tenant resolution (they set
-- app.system='on'; mirrors migrations 030/047/055). The client_secret is
-- stored here and never returned to a browser (handler strips it).

CREATE TABLE tenant_sso_configs (
    tenant_id UUID PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    issuer TEXT NOT NULL,
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,
    authorize_url TEXT NOT NULL,
    token_url TEXT NOT NULL,
    jwks_url TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE tenant_sso_configs ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_sso_configs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON tenant_sso_configs
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- The unauthenticated start/callback handlers resolve config cross-tenant
-- under app.system='on' (no app.tenant_id yet).
CREATE POLICY system_context_select ON tenant_sso_configs
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON tenant_sso_configs TO aulalite_app;
