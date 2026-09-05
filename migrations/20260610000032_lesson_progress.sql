-- Lesson-level completion tracking (learning-suite Cycle 2). A row means the
-- user explicitly marked the lesson complete (or a quiz auto-completed it in
-- a later cycle). Aggregates (course %, resume point) are computed on read.
CREATE TABLE lesson_completions (
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    lesson_id UUID NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    completed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (lesson_id, user_id)
);

CREATE INDEX lesson_completions_course_user_idx
    ON lesson_completions(course_id, user_id);
CREATE INDEX lesson_completions_user_recency_idx
    ON lesson_completions(user_id, completed_at DESC);

ALTER TABLE lesson_completions ENABLE ROW LEVEL SECURITY;
ALTER TABLE lesson_completions FORCE ROW LEVEL SECURITY;

-- Same model as the other course-scoped tables: tenant isolation at the row
-- level; per-role authorization (enrolled student writes own rows, teachers
-- read course-scoped) is enforced in the handlers.
CREATE POLICY lesson_completions_tenant_isolation ON lesson_completions
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
