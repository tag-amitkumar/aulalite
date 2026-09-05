-- Quizzes (learning-suite Cycle 3). A quiz lives at course level
-- (module_id NULL) or as a module item alongside lessons (module_id set,
-- ordered by sort_order). `mode` distinguishes graded assessments (attempt
-- caps, scores feed analytics) from practice self-checks (unlimited).
CREATE TABLE quizzes (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    module_id UUID REFERENCES modules(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    description TEXT,
    mode TEXT NOT NULL DEFAULT 'graded'
        CHECK (mode IN ('graded','practice')),
    time_limit_seconds INTEGER
        CHECK (time_limit_seconds IS NULL OR time_limit_seconds > 0),
    max_attempts INTEGER
        CHECK (max_attempts IS NULL OR max_attempts > 0),
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft','published','archived')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX quizzes_course_idx ON quizzes(course_id, status);
CREATE INDEX quizzes_module_idx ON quizzes(module_id, sort_order)
    WHERE module_id IS NOT NULL;

-- `prompt` stores the core-types `QuizPrompt` JSON (tagged by `kind`),
-- INCLUDING the answer key — student reads must project through the
-- student-safe view in the handlers, never select prompt verbatim.
CREATE TABLE quiz_questions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    quiz_id UUID NOT NULL REFERENCES quizzes(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    prompt_text TEXT NOT NULL,
    prompt JSONB NOT NULL,
    explanation TEXT NOT NULL DEFAULT '',
    points INTEGER NOT NULL DEFAULT 1 CHECK (points > 0)
);

CREATE INDEX quiz_questions_quiz_idx ON quiz_questions(quiz_id, position);

CREATE TABLE quiz_attempts (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    quiz_id UUID NOT NULL REFERENCES quizzes(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted_at TIMESTAMPTZ,
    score_points INTEGER,
    max_points INTEGER
);

CREATE INDEX quiz_attempts_quiz_user_idx ON quiz_attempts(quiz_id, user_id, started_at DESC);

CREATE TABLE quiz_attempt_answers (
    attempt_id UUID NOT NULL REFERENCES quiz_attempts(id) ON DELETE CASCADE,
    question_id UUID NOT NULL REFERENCES quiz_questions(id) ON DELETE CASCADE,
    answer JSONB NOT NULL,
    correct BOOLEAN NOT NULL,
    points_awarded INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (attempt_id, question_id)
);

ALTER TABLE quizzes ENABLE ROW LEVEL SECURITY;
ALTER TABLE quizzes FORCE ROW LEVEL SECURITY;
ALTER TABLE quiz_questions ENABLE ROW LEVEL SECURITY;
ALTER TABLE quiz_questions FORCE ROW LEVEL SECURITY;
ALTER TABLE quiz_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE quiz_attempts FORCE ROW LEVEL SECURITY;
ALTER TABLE quiz_attempt_answers ENABLE ROW LEVEL SECURITY;
ALTER TABLE quiz_attempt_answers FORCE ROW LEVEL SECURITY;

CREATE POLICY quizzes_tenant_isolation ON quizzes
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY quiz_questions_tenant_isolation ON quiz_questions
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY quiz_attempts_tenant_isolation ON quiz_attempts
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
-- Answers carry no tenant column; they are only reachable through their
-- attempt, which is tenant-isolated. Scope via EXISTS on the parent.
CREATE POLICY quiz_attempt_answers_via_attempt ON quiz_attempt_answers
    USING (EXISTS (
        SELECT 1 FROM quiz_attempts qa
        WHERE qa.id = attempt_id
          AND qa.tenant_id = current_setting('app.tenant_id', true)::uuid
    ));
