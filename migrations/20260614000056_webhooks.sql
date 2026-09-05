-- Outbound webhook subscriptions + delivery queue.
--
-- webhook_subscriptions: a tenant's registered endpoint URLs, the per-sub HMAC
-- signing secret, and the events it wants. webhook_deliveries: one row per
-- (event x subscription) enqueued for delivery, carrying the JSON payload,
-- attempt count, status, and last response code.
--
-- Tenant-scoped under RLS exactly like `announcements` for the admin CRUD +
-- emit_event enqueue paths (policy keys solely on app.tenant_id). The trusted
-- in-process delivery worker (services::webhook_delivery::run_delivery_worker)
-- runs cross-tenant under app.system='on' (db::begin_system_context, no tenant
-- GUC): it SELECTs+UPDATEs deliveries (claim 'sending' / record result) and
-- SELECTs subscriptions (url+secret join), so add the matching system_context
-- policies. Mirrors 20260530000030 / 20260614000047.

CREATE TABLE webhook_subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    url TEXT NOT NULL,
    -- Per-subscription HMAC-SHA256 signing secret (`whsec_...`). Shown once.
    secret TEXT NOT NULL,
    events TEXT[] NOT NULL DEFAULT '{}',
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX webhook_subscriptions_tenant_idx ON webhook_subscriptions (tenant_id);

ALTER TABLE webhook_subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE webhook_subscriptions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON webhook_subscriptions
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- The worker joins subscriptions for url+secret while claiming due deliveries.
CREATE POLICY system_context_select ON webhook_subscriptions
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON webhook_subscriptions TO aulalite_app;

CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    subscription_id UUID NOT NULL REFERENCES webhook_subscriptions(id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    payload_json JSONB NOT NULL,
    -- 'pending' | 'sending' | 'retrying' | 'delivered' | 'failed'
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    response_code INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX webhook_deliveries_tenant_idx ON webhook_deliveries (tenant_id);
-- Supports the worker's due-claim scan (status + created_at ordering) and the
-- per-subscription delivery log.
CREATE INDEX webhook_deliveries_status_created_idx ON webhook_deliveries (status, created_at);
CREATE INDEX webhook_deliveries_subscription_idx ON webhook_deliveries (subscription_id, created_at DESC);

ALTER TABLE webhook_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE webhook_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON webhook_deliveries
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- The worker claims (UPDATE -> 'sending'), records results (UPDATE), and the
-- claim CTE SELECTs the queue, all cross-tenant under app.system='on'.
CREATE POLICY system_context_select ON webhook_deliveries
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_update ON webhook_deliveries
    FOR UPDATE
    USING (current_setting('app.system', true) = 'on')
    WITH CHECK (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON webhook_deliveries TO aulalite_app;
