-- Phase 4 push notification hardening:
--   * delivery logs for email/push attempts, scoped by tenant under RLS
--   * device metadata for user self-service
--   * soft device revocation so old delivery rows remain understandable

ALTER TABLE device_tokens
    ADD COLUMN label TEXT,
    ADD COLUMN user_agent TEXT,
    ADD COLUMN revoked_at TIMESTAMPTZ;

CREATE INDEX device_tokens_user_active_idx
    ON device_tokens (tenant_id, user_id, last_seen_at DESC)
    WHERE revoked_at IS NULL;

CREATE TABLE notification_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    notification_id UUID REFERENCES notifications(id) ON DELETE SET NULL,
    channel TEXT NOT NULL CHECK (channel IN ('email','push')),
    provider TEXT NOT NULL,
    target_hash TEXT NOT NULL,
    target_label TEXT,
    device_token_id UUID REFERENCES device_tokens(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued','sent','failed','skipped')),
    provider_message_id TEXT,
    provider_status TEXT,
    error_code TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX notification_deliveries_tenant_created_idx
    ON notification_deliveries (tenant_id, created_at DESC);
CREATE INDEX notification_deliveries_user_created_idx
    ON notification_deliveries (tenant_id, user_id, created_at DESC);
CREATE INDEX notification_deliveries_status_idx
    ON notification_deliveries (tenant_id, status, created_at DESC);
CREATE INDEX notification_deliveries_device_idx
    ON notification_deliveries (device_token_id)
    WHERE device_token_id IS NOT NULL;

ALTER TABLE notification_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON notification_deliveries
    USING (tenant_id::text = current_setting('app.tenant_id', true))
    WITH CHECK (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON notification_deliveries TO aulalite_app;
