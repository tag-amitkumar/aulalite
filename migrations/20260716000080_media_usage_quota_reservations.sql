-- Enforce recording-storage quotas without holding a database transaction
-- open while an MP4 is uploaded to object storage. A short-lived reservation
-- accounts for bytes between the quota check and the atomic file_asset /
-- recording finalization.
--
-- Plan quota columns use -1 as the explicit "unlimited" sentinel. Existing
-- plans are positive and remain unchanged; zero means the feature is disabled.
ALTER TABLE plans
    ADD CONSTRAINT plans_class_minutes_quota_check
    CHECK (included_class_minutes >= -1),
    ADD CONSTRAINT plans_recording_gb_quota_check
    CHECK (included_recording_gb >= -1);

-- `created_at` is the recording's retention timestamp, not a processing lease
-- timestamp. Track status transitions independently so a manually retried old
-- recording is not immediately reclaimed as a stale worker.
ALTER TABLE recordings
    ADD COLUMN processing_updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

DROP INDEX IF EXISTS recordings_pending_idx;
CREATE INDEX recordings_pending_idx
    ON recordings (processing_status, processing_updated_at)
    WHERE processing_status IN ('pending', 'remuxing', 'uploading');

CREATE TABLE recording_storage_reservations (
    recording_id UUID PRIMARY KEY REFERENCES recordings(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    size_bytes BIGINT NOT NULL CHECK (size_bytes > 0),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX recording_storage_reservations_tenant_expiry_idx
    ON recording_storage_reservations (tenant_id, expires_at);

ALTER TABLE recording_storage_reservations ENABLE ROW LEVEL SECURITY;
ALTER TABLE recording_storage_reservations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON recording_storage_reservations
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid)
    WITH CHECK (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON recording_storage_reservations TO aulalite_app;

-- Supporting indexes for the two quota hot paths. INCLUDE keeps the monthly
-- class-minute aggregation index-only when visibility permits; the partial
-- recording index skips rows that cannot contribute storage bytes.
CREATE INDEX IF NOT EXISTS live_sessions_tenant_actual_started_quota_idx
    ON live_sessions (tenant_id, actual_started_at)
    INCLUDE (status, duration_minutes, actual_ended_at)
    WHERE actual_started_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS recordings_tenant_file_asset_quota_idx
    ON recordings (tenant_id, file_asset_id)
    WHERE file_asset_id IS NOT NULL;
