CREATE TABLE tenant_memberships (
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL
        CHECK (role IN ('org_admin','teacher','ta','student','parent')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','invited','suspended')),
    invited_by UUID REFERENCES users(id),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE INDEX tenant_memberships_user_idx ON tenant_memberships(user_id);
CREATE INDEX tenant_memberships_tenant_idx ON tenant_memberships(tenant_id);

ALTER TABLE tenant_memberships ENABLE ROW LEVEL SECURITY;

-- Allow access in two cases:
-- (a) the request is operating in tenant scope, or
-- (b) the request is bootstrapping before tenant resolution.
CREATE POLICY tenant_memberships_self_access ON tenant_memberships
    USING (
        tenant_id::text = current_setting('app.tenant_id', true)
        OR user_id::text = current_setting('app.user_id', true)
    );
