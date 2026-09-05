-- Student personal notes + lesson bookmarks. Both tenant-scoped under RLS exactly
-- like `announcements`/`lesson_completions` (policy keys solely on app.tenant_id),
-- so isolation holds under the non-bypass aulalite_app role (20260517000020_app_role.sql).
-- Per-user privacy (owner-only read/write) is enforced in the handlers/db layer via a
-- user_id = caller filter on every query; RLS handles cross-tenant isolation.

CREATE TABLE student_notes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lesson_id UUID NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, lesson_id)
);

CREATE INDEX student_notes_tenant_idx ON student_notes (tenant_id);
-- Supports the per-caller lesson fetch (WHERE user_id = $ AND lesson_id = $).
CREATE INDEX student_notes_user_lesson_idx ON student_notes (user_id, lesson_id);

ALTER TABLE student_notes ENABLE ROW LEVEL SECURITY;
ALTER TABLE student_notes FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON student_notes
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON student_notes TO aulalite_app;

CREATE TABLE lesson_bookmarks (
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lesson_id UUID NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, lesson_id)
);

CREATE INDEX lesson_bookmarks_tenant_idx ON lesson_bookmarks (tenant_id);
-- Supports the newest-first per-caller list (WHERE user_id = $ ORDER BY created_at DESC).
CREATE INDEX lesson_bookmarks_user_recency_idx ON lesson_bookmarks (user_id, created_at DESC);

ALTER TABLE lesson_bookmarks ENABLE ROW LEVEL SECURITY;
ALTER TABLE lesson_bookmarks FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON lesson_bookmarks
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON lesson_bookmarks TO aulalite_app;
