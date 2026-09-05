-- migrations/20260508000004_course_memberships.sql
CREATE TABLE course_memberships (
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL,
    role TEXT NOT NULL
        CHECK (role IN ('teacher','ta','student')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','removed')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (course_id, user_id)
);

CREATE INDEX course_memberships_user_idx ON course_memberships(user_id, tenant_id);
CREATE INDEX course_memberships_tenant_role_idx ON course_memberships(tenant_id, role);

ALTER TABLE course_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE course_memberships FORCE ROW LEVEL SECURITY;

CREATE POLICY course_memberships_tenant_isolation ON course_memberships
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
