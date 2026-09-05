-- migrations/20260508000010_audit_events.sql
CREATE TABLE audit_events (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    actor_user_id UUID NOT NULL REFERENCES users(id),
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id UUID NOT NULL,
    metadata JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_tenant_time_idx ON audit_events(tenant_id, occurred_at DESC);

ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_events FORCE ROW LEVEL SECURITY;

CREATE POLICY audit_events_tenant_isolation ON audit_events
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
