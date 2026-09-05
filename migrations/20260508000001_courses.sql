-- migrations/20260508000001_courses.sql
CREATE TABLE courses (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft','published','archived')),
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private')),
    cover_asset_id UUID,
    owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, slug)
);

CREATE INDEX courses_tenant_owner_idx ON courses(tenant_id, owner_user_id);
CREATE INDEX courses_tenant_status_idx ON courses(tenant_id, status);

ALTER TABLE courses ENABLE ROW LEVEL SECURITY;
ALTER TABLE courses FORCE ROW LEVEL SECURITY;

CREATE POLICY courses_tenant_isolation ON courses
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
