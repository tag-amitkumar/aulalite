-- migrations/20260509000016_assignments.sql
-- Phase 1c: assignments table. Course-level OR lesson-attached, per-assignment
-- grading_mode + late + lock_on_submit + accepted-types + release_mode policies.

CREATE TYPE assignment_grading_mode AS ENUM ('numeric', 'pass_fail');
CREATE TYPE assignment_release_mode AS ENUM ('instant', 'manual');
CREATE TYPE assignment_status AS ENUM ('draft', 'published');

CREATE TABLE assignments (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    lesson_id UUID REFERENCES lessons(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    instructions_md TEXT NOT NULL DEFAULT '',
    grading_mode assignment_grading_mode NOT NULL,
    max_points INTEGER,
    allow_late BOOLEAN NOT NULL DEFAULT TRUE,
    lock_on_submit BOOLEAN NOT NULL DEFAULT FALSE,
    accepts_text BOOLEAN NOT NULL DEFAULT TRUE,
    accepts_files BOOLEAN NOT NULL DEFAULT TRUE,
    release_mode assignment_release_mode NOT NULL DEFAULT 'instant',
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    due_at TIMESTAMPTZ,
    status assignment_status NOT NULL DEFAULT 'draft',
    published_at TIMESTAMPTZ,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (accepts_text OR accepts_files),
    CHECK (grading_mode <> 'numeric' OR max_points IS NOT NULL),
    CHECK (grading_mode <> 'pass_fail' OR max_points IS NULL),
    CHECK (max_points IS NULL OR max_points > 0)
);

CREATE INDEX idx_assignments_course
    ON assignments (tenant_id, course_id, status);
CREATE INDEX idx_assignments_lesson
    ON assignments (tenant_id, lesson_id) WHERE lesson_id IS NOT NULL;

ALTER TABLE assignments ENABLE ROW LEVEL SECURITY;
ALTER TABLE assignments FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON assignments
    USING (tenant_id::text = current_setting('app.tenant_id', true));
