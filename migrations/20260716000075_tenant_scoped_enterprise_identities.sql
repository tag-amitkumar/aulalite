-- Organization-controlled OIDC and LTI providers may assert an email address,
-- but that assertion is authoritative only inside the organization that owns
-- the provider configuration. Model that boundary explicitly instead of
-- treating every asserted address as a platform-global identity.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS identity_kind TEXT NOT NULL DEFAULT 'global',
    ADD COLUMN IF NOT EXISTS identity_tenant_id UUID REFERENCES tenants(id) ON DELETE RESTRICT;

ALTER TABLE users
    DROP CONSTRAINT IF EXISTS users_email_key,
    DROP CONSTRAINT IF EXISTS users_email_unique;

-- Platform-verified identities retain the historical one-account-per-email
-- rule. Tenant-scoped enterprise identities may share an email with a global
-- identity (or an identity in another tenant) without taking it over or
-- preventing the real user from signing up.
CREATE UNIQUE INDEX IF NOT EXISTS users_global_email_unique
    ON users (email)
    WHERE identity_tenant_id IS NULL;

CREATE INDEX IF NOT EXISTS users_identity_tenant_idx
    ON users (identity_tenant_id)
    WHERE identity_tenant_id IS NOT NULL;

ALTER TABLE users
    DROP CONSTRAINT IF EXISTS users_identity_scope_check;
ALTER TABLE users
    ADD CONSTRAINT users_identity_scope_check CHECK (
        (identity_kind = 'global' AND identity_tenant_id IS NULL)
        OR
        (identity_kind IN ('sso', 'lti') AND identity_tenant_id IS NOT NULL)
    );

-- Suspended organizations must not appear in the workspace switcher. Trialing
-- organizations remain usable; suspension is the explicit access cutoff.
CREATE OR REPLACE FUNCTION list_my_workspaces()
    RETURNS TABLE (
        tenant_id uuid,
        name text,
        slug text,
        role text
    )
    LANGUAGE sql
    STABLE
    SECURITY DEFINER
    SET search_path = public
AS $$
    SELECT tm.tenant_id, t.name, t.slug, tm.role
      FROM tenant_memberships tm
      JOIN tenants t ON t.id = tm.tenant_id
     WHERE tm.user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
       AND tm.status = 'active'
       AND t.status IN ('active', 'trialing')
     ORDER BY lower(t.name), tm.joined_at, tm.tenant_id
$$;

REVOKE ALL ON FUNCTION list_my_workspaces() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite_app;
