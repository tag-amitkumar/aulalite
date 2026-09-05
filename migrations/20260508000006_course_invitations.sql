-- migrations/20260508000006_course_invitations.sql
CREATE TABLE course_invitations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    email CITEXT NOT NULL,
    role TEXT NOT NULL
        CHECK (role IN ('teacher','ta','student')),
    token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','accepted','revoked','expired')),
    expires_at TIMESTAMPTZ NOT NULL,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    accepted_by UUID REFERENCES users(id),
    accepted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX course_invitations_pending_unique
    ON course_invitations(course_id, lower(email)) WHERE status = 'pending';

CREATE INDEX course_invitations_course_idx ON course_invitations(course_id);

ALTER TABLE course_invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE course_invitations FORCE ROW LEVEL SECURITY;

CREATE POLICY course_invitations_tenant_isolation ON course_invitations
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

CREATE OR REPLACE FUNCTION lookup_invitation_by_token(p_token TEXT)
    RETURNS TABLE(
        invitation_id UUID,
        tenant_id UUID,
        course_id UUID,
        email CITEXT,
        role TEXT,
        status TEXT,
        expires_at TIMESTAMPTZ
    )
    LANGUAGE sql
    STABLE
    SECURITY DEFINER
    SET search_path = public
AS $$
    SELECT id, tenant_id, course_id, email, role, status, expires_at
      FROM course_invitations
     WHERE token = p_token
$$;

REVOKE ALL ON FUNCTION lookup_invitation_by_token(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_invitation_by_token(TEXT) TO aulalite;
