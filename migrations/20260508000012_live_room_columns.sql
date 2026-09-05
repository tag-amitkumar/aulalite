-- migrations/20260508000012_live_room_columns.sql
-- Phase 1b-β: live class core — schema additions for transport mode,
-- screen-share path, and publish nonce.

ALTER TABLE live_session_series
    ADD COLUMN transport_mode TEXT NOT NULL DEFAULT 'webrtc'
    CHECK (transport_mode IN ('webrtc', 'hls'));

ALTER TABLE live_sessions
    ADD COLUMN screen_path TEXT,
    ADD COLUMN transport_mode TEXT NOT NULL DEFAULT 'webrtc'
    CHECK (transport_mode IN ('webrtc', 'hls')),
    ADD COLUMN publish_nonce TEXT,
    ADD COLUMN publish_nonce_expires_at TIMESTAMPTZ;

CREATE INDEX live_sessions_status_started_idx
    ON live_sessions (status, actual_started_at)
    WHERE status = 'live';
