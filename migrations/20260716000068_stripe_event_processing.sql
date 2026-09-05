-- Make Stripe idempotency completion-aware. Previously the event id was
-- inserted before business mutations, so any transient failure caused every
-- provider retry to no-op forever.

ALTER TABLE stripe_events
    ADD COLUMN processing_started_at TIMESTAMPTZ,
    ADD COLUMN processed_at TIMESTAMPTZ,
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN last_error TEXT;

-- Rows created by the old implementation were treated as fully processed.
UPDATE stripe_events
   SET processed_at = received_at,
       attempt_count = 1;

CREATE INDEX stripe_events_retryable_idx
    ON stripe_events (processing_started_at)
    WHERE processed_at IS NULL;

GRANT UPDATE ON stripe_events TO aulalite_app;
