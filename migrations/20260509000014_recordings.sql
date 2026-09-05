-- migrations/20260509000014_recordings.sql
-- Phase 1b-δ: live class recording. One recordings row per session, linked to
-- the produced MP4 file_asset. Tenant-isolated via RLS.

CREATE TABLE recordings (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    file_asset_id UUID REFERENCES file_assets(id),
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER NOT NULL CHECK (duration_seconds > 0),
    processing_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (processing_status IN ('pending','remuxing','uploading','available','failed')),
    processing_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id)
);
CREATE INDEX recordings_pending_idx
    ON recordings (processing_status, created_at)
    WHERE processing_status IN ('pending','remuxing','uploading');

ALTER TABLE recordings ENABLE ROW LEVEL SECURITY;
ALTER TABLE recordings FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON recordings
    USING (tenant_id::text = current_setting('app.tenant_id', true));
