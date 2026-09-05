-- Plagiarism detection: per-submission text fingerprints (word k-shingle hashes)
-- compared within an assignment. Tenant-scoped under RLS like `announcements`
-- (policy keys solely on app.tenant_id), enforced under the non-bypass
-- `aulalite_app` role.
CREATE TABLE submission_fingerprints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    submission_id UUID NOT NULL UNIQUE REFERENCES submissions(id) ON DELETE CASCADE,
    assignment_id UUID NOT NULL REFERENCES assignments(id) ON DELETE CASCADE,
    student_user_id UUID NOT NULL REFERENCES users(id),
    shingles BIGINT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX submission_fingerprints_tenant_idx ON submission_fingerprints (tenant_id);
CREATE INDEX submission_fingerprints_assignment_idx ON submission_fingerprints (assignment_id, created_at);
ALTER TABLE submission_fingerprints ENABLE ROW LEVEL SECURITY;
ALTER TABLE submission_fingerprints FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON submission_fingerprints USING (tenant_id::text = current_setting('app.tenant_id', true));
GRANT SELECT, INSERT, UPDATE, DELETE ON submission_fingerprints TO aulalite_app;
