-- Serialize tenant Checkout creation without holding a database transaction
-- open across Stripe network calls. Every tenant has at most one current
-- intent. Requests for the same plan reuse its idempotency key/session, while
-- an expired row can be atomically replaced with a fresh intent.

CREATE TABLE stripe_checkout_intents (
    tenant_id UUID PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    plan_id TEXT NOT NULL REFERENCES plans(id),
    customer_email TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    stripe_session_id TEXT,
    checkout_url TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT stripe_checkout_intents_session_pair CHECK (
        (stripe_session_id IS NULL AND checkout_url IS NULL)
        OR (stripe_session_id IS NOT NULL AND checkout_url IS NOT NULL)
    )
);

CREATE INDEX stripe_checkout_intents_expiry
    ON stripe_checkout_intents (expires_at);

ALTER TABLE stripe_checkout_intents ENABLE ROW LEVEL SECURITY;
ALTER TABLE stripe_checkout_intents FORCE ROW LEVEL SECURITY;

CREATE POLICY stripe_checkout_intents_tenant_isolation
    ON stripe_checkout_intents
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid)
    WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON stripe_checkout_intents TO aulalite_app;
