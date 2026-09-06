-- Whether a finished recording actually contains a video stream.
--
-- Classes recorded before the H.264 publish preference landed were published as
-- VP8. The recording pipeline remuxes segments with `ffmpeg -c copy`, and the
-- MP4 muxer cannot carry VP8, so the video track was dropped at remux time and
-- only the Opus audio survived. Those objects are audio-only permanently -- the
-- source segments are long gone under MediaMTX's 24h recordDeleteAfter -- and
-- the player rendered them as a black rectangle with working sound, which reads
-- as "the video is broken" rather than "this one has no video".
--
-- Nullable on purpose, and NULL means "not determined", not "no video":
--   * every row that predates this column starts NULL until the backfill probes
--     the stored object,
--   * a probe that fails leaves NULL rather than guessing.
-- The UI only shows its audio-only treatment for an explicit FALSE, so an
-- unknown row renders exactly as it does today and a healthy recording can
-- never be mislabelled by a failed probe.
ALTER TABLE recordings ADD COLUMN IF NOT EXISTS has_video BOOLEAN;

COMMENT ON COLUMN recordings.has_video IS
    'TRUE/FALSE once the produced MP4 has been probed for a video stream; NULL when not yet determined (pre-existing rows, or a probe that failed).';
