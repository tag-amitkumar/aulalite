-- Real-disconnection tracking for live sessions.
--
-- The auto-end sweep used to force-end a class purely on elapsed wall clock
-- (actual_started_at + duration + grace), which terminated classes that were
-- healthy and actively publishing. Cleanup is now driven by whether the media
-- server still reports a publisher on the session's path, and that needs one
-- durable timestamp so "the publisher has been gone a while" survives a
-- backend restart.
--
-- NULL means "not yet observed". The sweep seeds it from actual_started_at the
-- first time it sees the path inactive, refreshes it to now() while the path is
-- active, and only ends a session once the publisher has been continuously
-- absent for the confirmation window.
ALTER TABLE live_sessions
    ADD COLUMN IF NOT EXISTS publisher_last_seen_at timestamptz;

-- The sweep scans live rows every tick; keep that scan cheap as the table grows.
CREATE INDEX IF NOT EXISTS live_sessions_live_publisher_idx
    ON live_sessions (publisher_last_seen_at)
    WHERE status = 'live';
