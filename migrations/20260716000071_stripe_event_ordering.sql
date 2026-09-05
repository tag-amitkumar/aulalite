-- Stripe does not guarantee webhook delivery order. Persist the provider's
-- event timestamp on the authoritative subscription row so a delayed older
-- event cannot overwrite a newer cancellation, payment failure, or upgrade.

ALTER TABLE subscriptions
    ADD COLUMN stripe_event_created_at TIMESTAMPTZ;

CREATE UNIQUE INDEX subscriptions_stripe_subscription_unique
    ON subscriptions (stripe_subscription_id)
    WHERE stripe_subscription_id IS NOT NULL;
