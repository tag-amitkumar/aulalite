-- Assignment peer review. Three tenant-scoped tables under RLS exactly like
-- `rubrics` / `announcements` (policy keys solely on app.tenant_id), enforced
-- under the non-bypass `aulalite_app` role (20260517000020_app_role.sql).

CREATE TABLE peer_review_configs (
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    reviews_per_student INTEGER NOT NULL CHECK (reviews_per_student >= 1),
    -- Optional reuse of the assignment's existing rubric (rubrics.id). The
    -- handler verifies the rubric belongs to this assignment before storing.
    rubric_id UUID REFERENCES rubrics(id) ON DELETE SET NULL,
    anonymous BOOLEAN NOT NULL DEFAULT true,
    due_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One config per assignment (the data layer UPSERTs on this key).
    PRIMARY KEY (assignment_id)
);
CREATE INDEX peer_review_configs_tenant_idx ON peer_review_configs (tenant_id);
CREATE INDEX peer_review_configs_course_idx ON peer_review_configs (course_id);

ALTER TABLE peer_review_configs ENABLE ROW LEVEL SECURITY;
ALTER TABLE peer_review_configs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON peer_review_configs
    USING (tenant_id::text = current_setting('app.tenant_id', true));
GRANT SELECT, INSERT, UPDATE, DELETE ON peer_review_configs TO aulalite_app;

CREATE TABLE peer_review_allocations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    reviewer_user_id UUID NOT NULL REFERENCES users(id),
    submission_id UUID NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
    -- 'pending' | 'submitted' (plain text, no custom enum needed).
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A reviewer is never assigned the same submission twice.
    UNIQUE (assignment_id, reviewer_user_id, submission_id)
);
CREATE INDEX peer_review_allocations_tenant_idx ON peer_review_allocations (tenant_id);
-- Supports the reviewer-queue query (WHERE assignment_id AND reviewer_user_id).
CREATE INDEX peer_review_allocations_reviewer_idx
    ON peer_review_allocations (assignment_id, reviewer_user_id);
-- Supports the staff aggregate + received-by-author join via the submission.
CREATE INDEX peer_review_allocations_submission_idx ON peer_review_allocations (submission_id);

ALTER TABLE peer_review_allocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE peer_review_allocations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON peer_review_allocations
    USING (tenant_id::text = current_setting('app.tenant_id', true));
GRANT SELECT, INSERT, UPDATE, DELETE ON peer_review_allocations TO aulalite_app;

CREATE TABLE peer_reviews (
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    -- One filed review per allocation; re-filing UPSERTs on this key.
    allocation_id UUID NOT NULL REFERENCES peer_review_allocations(id) ON DELETE CASCADE,
    scores_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    comment_md TEXT NOT NULL DEFAULT '',
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (allocation_id)
);
CREATE INDEX peer_reviews_tenant_idx ON peer_reviews (tenant_id);

ALTER TABLE peer_reviews ENABLE ROW LEVEL SECURITY;
ALTER TABLE peer_reviews FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON peer_reviews
    USING (tenant_id::text = current_setting('app.tenant_id', true));
GRANT SELECT, INSERT, UPDATE, DELETE ON peer_reviews TO aulalite_app;
