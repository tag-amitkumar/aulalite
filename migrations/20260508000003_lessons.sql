-- migrations/20260508000003_lessons.sql
CREATE TABLE lessons (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    module_id UUID NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
    type TEXT NOT NULL
        CHECK (type IN ('rich_text','video','live_session','file_bundle')),
    title TEXT NOT NULL,
    body_md TEXT,
    video_asset_id UUID,
    live_session_id UUID,
    sort_order INTEGER NOT NULL,
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX lessons_module_sort_idx ON lessons(module_id, sort_order);

ALTER TABLE lessons ENABLE ROW LEVEL SECURITY;
ALTER TABLE lessons FORCE ROW LEVEL SECURITY;

CREATE POLICY lessons_tenant_isolation ON lessons
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
