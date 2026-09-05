-- Course discussion forums / Q&A. Two tables scoped to (tenant_id, course_id),
-- both tenant-isolated under RLS exactly like `announcements` (policy keys solely
-- on app.tenant_id), enforced under the non-bypass `aulalite_app` role
-- (20260517000020_app_role.sql).

CREATE TABLE discussions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    author_user_id UUID NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    body_md TEXT NOT NULL,
    pinned BOOLEAN NOT NULL DEFAULT false,
    locked BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX discussions_tenant_idx ON discussions (tenant_id);
-- Supports the list query: WHERE course_id = $1 ORDER BY pinned DESC, created_at DESC.
CREATE INDEX discussions_course_pinned_created_idx
    ON discussions (course_id, pinned DESC, created_at DESC);

ALTER TABLE discussions ENABLE ROW LEVEL SECURITY;
ALTER TABLE discussions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON discussions
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON discussions TO aulalite_app;

CREATE TABLE discussion_posts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    discussion_id UUID NOT NULL REFERENCES discussions(id) ON DELETE CASCADE,
    -- Self-FK for nested replies; NULL = top-level reply. Deleting a parent
    -- cascades its descendants.
    parent_post_id UUID REFERENCES discussion_posts(id) ON DELETE CASCADE,
    author_user_id UUID NOT NULL REFERENCES users(id),
    body_md TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX discussion_posts_tenant_idx ON discussion_posts (tenant_id);
-- Supports listing a thread's posts oldest-first.
CREATE INDEX discussion_posts_thread_created_idx
    ON discussion_posts (discussion_id, created_at ASC);
CREATE INDEX discussion_posts_parent_idx ON discussion_posts (parent_post_id);

ALTER TABLE discussion_posts ENABLE ROW LEVEL SECURITY;
ALTER TABLE discussion_posts FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON discussion_posts
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON discussion_posts TO aulalite_app;
