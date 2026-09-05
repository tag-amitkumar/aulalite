CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'trialing'
        CHECK (status IN ('active','trialing','suspended')),
    plan_id TEXT,
    stripe_customer_id TEXT,
    branding JSONB,
    recording_default BOOLEAN NOT NULL DEFAULT TRUE,
    recording_retention_days INTEGER NOT NULL DEFAULT 90
        CHECK (recording_retention_days BETWEEN 1 AND 3650),
    trial_ends_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX tenants_status_idx ON tenants(status);

ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenants_self_access ON tenants
    USING (id = current_setting('app.tenant_id', true)::uuid);
