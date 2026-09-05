-- Certificates (learning-suite Cycle 5): completion makes a student ELIGIBLE;
-- a teacher ISSUES (or revokes) the certificate. Issued certificates carry a
-- public credential id verifiable without authentication via the
-- SECURITY DEFINER lookup below.
CREATE TABLE certificates (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Set when issued (e.g. AULA-1A2B-3C4D); stable across revoke/re-issue.
    credential_id TEXT UNIQUE,
    status TEXT NOT NULL DEFAULT 'eligible'
        CHECK (status IN ('eligible','issued','revoked')),
    -- Snapshots taken at issue time so later renames don't rewrite history.
    recipient_name TEXT,
    course_title TEXT,
    issued_by UUID REFERENCES users(id),
    issued_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (course_id, user_id)
);

CREATE INDEX certificates_user_idx ON certificates(user_id, created_at DESC);
CREATE INDEX certificates_course_idx ON certificates(course_id, status);

ALTER TABLE certificates ENABLE ROW LEVEL SECURITY;
ALTER TABLE certificates FORCE ROW LEVEL SECURITY;

CREATE POLICY certificates_tenant_isolation ON certificates
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Public verification: bypasses RLS to resolve a credential id, returning ONLY
-- the public fields. Revoked certificates resolve with status so verifiers see
-- an explicit "revoked" rather than a silent 404.
CREATE OR REPLACE FUNCTION verify_certificate(p_credential_id TEXT)
    RETURNS TABLE(
        credential_id TEXT,
        status TEXT,
        recipient_name TEXT,
        course_title TEXT,
        issued_at TIMESTAMPTZ
    )
    LANGUAGE sql
    STABLE
    SECURITY DEFINER
    SET search_path = public
AS $$
    SELECT credential_id, status, recipient_name, course_title, issued_at
      FROM certificates
     WHERE credential_id = p_credential_id
       AND status IN ('issued','revoked')
$$;
