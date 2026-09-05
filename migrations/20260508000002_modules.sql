-- migrations/20260508000002_modules.sql
CREATE TABLE modules (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX modules_course_sort_idx ON modules(course_id, sort_order);

ALTER TABLE modules ENABLE ROW LEVEL SECURITY;
ALTER TABLE modules FORCE ROW LEVEL SECURITY;

CREATE POLICY modules_tenant_isolation ON modules
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
