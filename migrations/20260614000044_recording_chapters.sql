-- Recording chapters: named timestamp bookmarks into a recording's timeline.
-- One row per (recording, label, position). Authored/deleted by course staff
-- (handler-gated on caller_can_staff_course); read by anyone who can read the
-- course (handler-gated) to seek the replay <video>. Tenant-scoped under RLS
-- exactly like `recordings` (policy keys solely on app.tenant_id), so it's
-- enforced under the non-bypass `aulalite_app` role (20260517000020_app_role.sql).

CREATE TABLE recording_chapters (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    recording_id UUID NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    position_seconds INTEGER NOT NULL CHECK (position_seconds >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX recording_chapters_tenant_idx ON recording_chapters (tenant_id);
-- Supports the timeline-ordered list query (WHERE recording_id = $1 ORDER BY position_seconds).
CREATE INDEX recording_chapters_recording_pos_idx
    ON recording_chapters (recording_id, position_seconds);

ALTER TABLE recording_chapters ENABLE ROW LEVEL SECURITY;
ALTER TABLE recording_chapters FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON recording_chapters
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Match the explicit table grants the app role relies on (idempotent with the
-- default privileges from 20260517000020_app_role.sql; keeps the migration
-- self-contained).
GRANT SELECT, INSERT, UPDATE, DELETE ON recording_chapters TO aulalite_app;
