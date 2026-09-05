-- migrations/20260508000005_enrollment_codes.sql
CREATE TABLE enrollment_codes (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    code TEXT NOT NULL UNIQUE,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX enrollment_codes_course_idx ON enrollment_codes(course_id);

ALTER TABLE enrollment_codes ENABLE ROW LEVEL SECURITY;
ALTER TABLE enrollment_codes FORCE ROW LEVEL SECURITY;

CREATE POLICY enrollment_codes_tenant_isolation ON enrollment_codes
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Cross-tenant lookup helper. SECURITY DEFINER bypasses RLS for the code-resolution
-- step; the caller MUST then SET LOCAL app.tenant_id = <returned tenant_id> before
-- any further DB work, or RLS will continue to hide tenant data.
CREATE OR REPLACE FUNCTION lookup_enrollment_code(p_code TEXT)
    RETURNS TABLE(
        code_id UUID,
        tenant_id UUID,
        course_id UUID,
        max_uses INTEGER,
        uses INTEGER,
        expires_at TIMESTAMPTZ
    )
    LANGUAGE sql
    STABLE
    SECURITY DEFINER
    SET search_path = public
AS $$
    SELECT id, tenant_id, course_id, max_uses, uses, expires_at
      FROM enrollment_codes
     WHERE code = p_code
$$;

REVOKE ALL ON FUNCTION lookup_enrollment_code(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_enrollment_code(TEXT) TO aulalite;
