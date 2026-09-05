-- migrations/20260529000024_billing.sql
-- Billing data foundation: plans, subscriptions, and the Stripe webhook
-- idempotency ledger. No HTTP routes yet — this migration only lands the
-- schema the billing service + db layer build on.
--
-- Three tables with deliberately different scoping:
--   * plans          — GLOBAL catalog. No tenant_id, no RLS: every tenant must
--                      be able to read the price/quota of any plan (e.g. to
--                      render an upgrade prompt). Seeded with two tiers.
--   * subscriptions  — TENANT-SCOPED (PK = tenant_id). Strict tenant_isolation
--                      RLS + FORCE, mirroring the parent_links pattern, so the
--                      non-bypass `aulalite_app` role only ever sees its own
--                      tenant's row.
--   * stripe_events  — GLOBAL idempotency ledger keyed by the Stripe event id.
--                      The webhook handler INSERTs each event id ON CONFLICT DO
--                      NOTHING; a conflict means "already processed". No RLS:
--                      Stripe events arrive server-to-server with no tenant GUC,
--                      and the id namespace is Stripe-global.
--
-- The `tenants` table already carries (unused) plan_id + stripe_customer_id
-- columns (see 20260503000002_tenants.sql); those remain the denormalized
-- pointers, while `subscriptions` is the authoritative per-tenant billing row.
-- Usage is computed on-read from existing tables, so NO usage_counters table.

-- ---------------------------------------------------------------------------
-- plans — global catalog (no tenant_id, no RLS, readable by all)
-- ---------------------------------------------------------------------------
CREATE TABLE plans (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    monthly_price_cents INTEGER NOT NULL,
    included_seats INTEGER NOT NULL,
    included_class_minutes INTEGER NOT NULL,
    included_recording_gb INTEGER NOT NULL,
    stripe_price_id TEXT
);

-- Seed the two MVP tiers (Section 1 of the scope doc: "2 tiers"). Prices and
-- quotas are sensible placeholders pending the pricing sheet (open question
-- #2 in the spec); the Stripe price ids are NULL until wired in test mode.
INSERT INTO plans (id, name, monthly_price_cents, included_seats, included_class_minutes, included_recording_gb, stripe_price_id)
VALUES
    ('starter', 'Starter', 4900,  50,  10000, 50,  NULL),
    ('pro',     'Pro',     14900, 250, 60000, 250, NULL);

-- plans is global; grant read to the app role explicitly (it has no tenant_id
-- to gate on, and the default-privileges grant from 20260517000020 also covers
-- it, but we state SELECT here for intent + forward-compat).
GRANT SELECT ON plans TO aulalite_app;

-- ---------------------------------------------------------------------------
-- subscriptions — one row per tenant (PK = tenant_id), tenant-isolated RLS
-- ---------------------------------------------------------------------------
CREATE TABLE subscriptions (
    tenant_id UUID PRIMARY KEY REFERENCES tenants(id),
    plan_id TEXT NOT NULL REFERENCES plans(id),
    status TEXT NOT NULL DEFAULT 'trialing'
        CHECK (status IN ('trialing','active','past_due','canceled')),
    current_period_start TIMESTAMPTZ,
    current_period_end TIMESTAMPTZ,
    trial_ends_at TIMESTAMPTZ,
    stripe_subscription_id TEXT,
    overage_behavior TEXT NOT NULL DEFAULT 'block'
        CHECK (overage_behavior IN ('block','metered')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE subscriptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE subscriptions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON subscriptions
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON subscriptions TO aulalite_app;

-- ---------------------------------------------------------------------------
-- stripe_events — global idempotency ledger (no RLS)
-- ---------------------------------------------------------------------------
CREATE TABLE stripe_events (
    id TEXT PRIMARY KEY,            -- Stripe event id (evt_...)
    event_type TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

GRANT SELECT, INSERT ON stripe_events TO aulalite_app;
