-- migrations/20260509000017_submissions.sql
-- Phase 1c: submissions table. One row per (assignment, student). Drives
-- the draft -> submitted -> graded/returned state machine.

CREATE TYPE submission_status AS ENUM ('draft', 'submitted', 'returned', 'graded');

CREATE TABLE submissions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    student_user_id UUID NOT NULL REFERENCES users(id),
    status submission_status NOT NULL DEFAULT 'draft',
    text_answer TEXT,
    attachment_asset_ids UUID[] NOT NULL DEFAULT '{}',
    submitted_at TIMESTAMPTZ,
    is_late BOOLEAN NOT NULL DEFAULT FALSE,
    numeric_grade NUMERIC(7,2),
    letter_grade TEXT,
    passed BOOLEAN,
    student_visible_feedback TEXT,
    teacher_only_notes TEXT,
    graded_by_user_id UUID REFERENCES users(id),
    graded_at TIMESTAMPTZ,
    released_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (assignment_id, student_user_id)
);

CREATE INDEX idx_submissions_student
    ON submissions (tenant_id, student_user_id, status);
CREATE INDEX idx_submissions_assignment
    ON submissions (tenant_id, assignment_id, status);

ALTER TABLE submissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE submissions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON submissions
    USING (tenant_id::text = current_setting('app.tenant_id', true));
