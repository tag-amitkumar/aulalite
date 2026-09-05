-- Course announcements. One row per announcement, scoped to (tenant_id,
-- course_id). Authored by course staff (handler-gated on caller_can_staff_course);
-- readable by anyone who can read the course (handler-gated). Tenant-scoped under
-- RLS exactly like `attendance` (policy keys solely on app.tenant_id), so it is
-- enforced under the non-bypass `aulalite_app` role (20260517000020_app_role.sql).

CREATE TABLE announcements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    author_user_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    body_md TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX announcements_tenant_idx ON announcements (tenant_id);
-- Supports the newest-first list query (WHERE course_id = $1 ORDER BY created_at DESC).
CREATE INDEX announcements_course_created_idx ON announcements (course_id, created_at DESC);

ALTER TABLE announcements ENABLE ROW LEVEL SECURITY;
ALTER TABLE announcements FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON announcements
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Match the explicit table grants the app role relies on (idempotent with the
-- default privileges from 20260517000020_app_role.sql; keeps the migration
-- self-contained).
GRANT SELECT, INSERT, UPDATE, DELETE ON announcements TO aulalite_app;
