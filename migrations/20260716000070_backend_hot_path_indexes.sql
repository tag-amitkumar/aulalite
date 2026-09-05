-- Targeted indexes for production hot paths. These match the exact equality
-- prefixes and ordering used by the webhook delivery log/worker, parent
-- attendance history, and Stripe's server-to-server customer lookup.

-- Admin delivery logs are tenant-scoped and keyset-stable by created_at + id.
CREATE INDEX webhook_deliveries_tenant_recent_idx
    ON webhook_deliveries (tenant_id, created_at DESC, id DESC);

CREATE INDEX webhook_deliveries_tenant_subscription_recent_idx
    ON webhook_deliveries (tenant_id, subscription_id, created_at DESC, id DESC);

CREATE INDEX webhook_subscriptions_tenant_recent_idx
    ON webhook_subscriptions (tenant_id, created_at DESC, id DESC);

-- The worker ignores terminal history. Keeping a small ordered partial index
-- lets its oldest-first LIMIT scan stop early without traversing the much
-- larger delivered/failed population.
CREATE INDEX webhook_deliveries_claimable_created_idx
    ON webhook_deliveries (created_at ASC, id ASC)
    WHERE status IN ('pending', 'retrying', 'sending');

-- Parent dashboards filter by tenant + child and sort recent attendance.
CREATE INDEX attendance_tenant_user_recent_idx
    ON attendance (tenant_id, user_id, first_joined_at DESC, session_id);

-- Stripe webhooks resolve a tenant from a customer id without tenant context.
CREATE INDEX tenants_stripe_customer_id_idx
    ON tenants (stripe_customer_id)
    WHERE stripe_customer_id IS NOT NULL;
