-- Rubric-based grading. A rubric is attached 1:1 to an assignment via
-- rubrics.assignment_id (the assignments table is never altered). Each rubric
-- owns ordered rubric_criteria; per-submission per-criterion scores live in
-- submission_criterion_scores. All three are tenant-scoped under RLS exactly
-- like `announcements` (policy keys solely on app.tenant_id), enforced under
-- the non-bypass `aulalite_app` role (20260517000020_app_role.sql).

CREATE TABLE rubrics (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One rubric per assignment; the data layer create-or-replaces by deleting
    -- the existing row first, but the constraint guards against duplicates.
    UNIQUE (assignment_id)
);
CREATE INDEX rubrics_tenant_idx ON rubrics (tenant_id);
CREATE INDEX rubrics_course_idx ON rubrics (course_id);

CREATE TABLE rubric_criteria (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    rubric_id UUID NOT NULL REFERENCES rubrics(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    max_points INTEGER NOT NULL CHECK (max_points > 0),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rubric_criteria_tenant_idx ON rubric_criteria (tenant_id);
-- Supports the ordered criteria list query (WHERE rubric_id = $1 ORDER BY sort_order).
CREATE INDEX rubric_criteria_rubric_order_idx ON rubric_criteria (rubric_id, sort_order);

CREATE TABLE submission_criterion_scores (
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    submission_id UUID NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
    criterion_id UUID NOT NULL REFERENCES rubric_criteria(id) ON DELETE CASCADE,
    points NUMERIC NOT NULL CHECK (points >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (submission_id, criterion_id)
);
CREATE INDEX submission_criterion_scores_tenant_idx ON submission_criterion_scores (tenant_id);
CREATE INDEX submission_criterion_scores_criterion_idx ON submission_criterion_scores (criterion_id);

ALTER TABLE rubrics ENABLE ROW LEVEL SECURITY;
ALTER TABLE rubrics FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON rubrics
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE rubric_criteria ENABLE ROW LEVEL SECURITY;
ALTER TABLE rubric_criteria FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON rubric_criteria
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE submission_criterion_scores ENABLE ROW LEVEL SECURITY;
ALTER TABLE submission_criterion_scores FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON submission_criterion_scores
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Explicit table grants the non-bypass app role relies on (idempotent with the
-- default privileges from 20260517000020_app_role.sql; keeps this migration
-- self-contained).
GRANT SELECT, INSERT, UPDATE, DELETE ON rubrics TO aulalite_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON rubric_criteria TO aulalite_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON submission_criterion_scores TO aulalite_app;
