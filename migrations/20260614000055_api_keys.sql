-- Tenant-scoped programmatic API keys for the read-only public API (/api/v1/*).
-- A key's PLAINTEXT secret is shown to the admin exactly once at mint; we store
-- only its SHA-256 hash. The `prefix` is the public, indexed half used to
-- resolve the key during authentication (db::api_keys::authenticate), which
-- runs with NO app.tenant_id (the tenant is unknown until resolved) and so
-- relies on the permissive system_context policies below (app.system='on').
--
-- Tenant-scoped under RLS exactly like `announcements` (policy keys solely on
-- app.tenant_id) for the admin mint/list/revoke path; PLUS system_context
-- SELECT (prefix lookup) + UPDATE (last_used_at bump) for the trusted in-process
-- authenticate path that sets app.system='on'. Mirrors 20260614000047.

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL,
    -- Public, indexed identifier embedded in the token: `ak_<prefix>`.
    prefix TEXT NOT NULL UNIQUE,
    -- Lowercase hex SHA-256 of the secret half. NEVER the plaintext.
    key_hash TEXT NOT NULL,
    scopes TEXT[] NOT NULL DEFAULT '{}',
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX api_keys_tenant_idx ON api_keys (tenant_id);
-- Authentication resolves by prefix (already UNIQUE, so the unique index serves
-- the equality lookup).

ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON api_keys
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- authenticate() runs cross-tenant under app.system='on' (no tenant GUC): it
-- SELECTs by prefix to resolve the key and UPDATEs last_used_at on a hit.
CREATE POLICY system_context_select ON api_keys
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_update ON api_keys
    FOR UPDATE
    USING (current_setting('app.system', true) = 'on')
    WITH CHECK (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON api_keys TO aulalite_app;
