-- migrations/20260529000023_parent_links.sql
-- Parent role data layer.
--
-- Two tables:
--   * parent_links        — a confirmed parent<->student relationship within a
--                           tenant. One row per (parent_user_id, student_user_id).
--   * parent_invitations  — a staff-issued invite for a parent email to be linked
--                           to a student. Becomes a `parent_links` row + a
--                           `tenant_memberships` row (role='parent') when the
--                           invited email first signs in (JIT acceptance).
--
-- Acceptance subtlety (see accept_parent_invitations_for_email below): the invited
-- parent's account is provisioned the FIRST time they sign in, INSIDE the JIT
-- provisioning transaction, which has NO per-request tenant GUC set and may need
-- to write rows for SEVERAL tenants at once (a parent invited by multiple schools).
-- The strict `tenant_isolation` RLS policy below cannot be satisfied by a single
-- `app.tenant_id` GUC in that situation. We therefore mirror the codebase's
-- established cross-tenant escape — a SECURITY DEFINER function owned by the
-- migration role (cf. lookup_enrollment_code / lookup_invitation_by_token) — to
-- perform the acceptance writes server-side, bypassing RLS for that one step.

-- ---------------------------------------------------------------------------
-- parent_links
-- ---------------------------------------------------------------------------
CREATE TABLE parent_links (
    parent_user_id UUID NOT NULL REFERENCES users(id),
    student_user_id UUID NOT NULL REFERENCES users(id),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    relationship TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (parent_user_id, student_user_id)
);

CREATE INDEX parent_links_tenant_idx ON parent_links (tenant_id);
CREATE INDEX parent_links_student_idx ON parent_links (student_user_id);

ALTER TABLE parent_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE parent_links FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON parent_links
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON parent_links TO aulalite_app;

-- ---------------------------------------------------------------------------
-- parent_invitations
-- ---------------------------------------------------------------------------
CREATE TABLE parent_invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    parent_email CITEXT NOT NULL,
    student_user_id UUID NOT NULL REFERENCES users(id),
    relationship TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','accepted','revoked')),
    created_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at TIMESTAMPTZ
);

-- At most one PENDING invite per (tenant, parent email, student). Mirrors the
-- partial-unique on course_invitations and lets create_invitation be idempotent.
CREATE UNIQUE INDEX parent_invitations_pending_unique
    ON parent_invitations (tenant_id, lower(parent_email), student_user_id)
    WHERE status = 'pending';

CREATE INDEX parent_invitations_tenant_idx ON parent_invitations (tenant_id);
CREATE INDEX parent_invitations_student_idx ON parent_invitations (student_user_id);

ALTER TABLE parent_invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE parent_invitations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON parent_invitations
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON parent_invitations TO aulalite_app;

-- ---------------------------------------------------------------------------
-- Cross-tenant acceptance helper.
--
-- Runs at JIT-provisioning time, where the invited parent has just been created
-- and there is no per-request tenant context. For EVERY pending invitation
-- matching the signed-in email (across ALL tenants) it:
--   1. inserts the parent_links row,
--   2. upserts an active tenant_memberships row with role 'parent',
--   3. flips the invitation to 'accepted'.
-- Returns the number of invitations accepted.
--
-- SECURITY DEFINER so the writes run with the table-owning migration role's
-- privileges and bypass the strict tenant_isolation policies (we can't set a
-- single app.tenant_id when several tenants are involved). This mirrors the
-- existing SECURITY DEFINER lookup helpers; it is the codebase's sanctioned way
-- to do cross-tenant work that precedes per-request tenant resolution.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION accept_parent_invitations_for_email(
        p_user_id UUID,
        p_email TEXT
    )
    RETURNS INTEGER
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
DECLARE
    v_count INTEGER := 0;
    r RECORD;
BEGIN
    FOR r IN
        SELECT id, tenant_id, student_user_id, relationship
          FROM parent_invitations
         WHERE status = 'pending'
           AND lower(parent_email::text) = lower(p_email)
    LOOP
        INSERT INTO parent_links
            (parent_user_id, student_user_id, tenant_id, relationship)
        VALUES (p_user_id, r.student_user_id, r.tenant_id, r.relationship)
        ON CONFLICT (parent_user_id, student_user_id) DO NOTHING;

        INSERT INTO tenant_memberships
            (tenant_id, user_id, role, status)
        VALUES (r.tenant_id, p_user_id, 'parent', 'active')
        ON CONFLICT (tenant_id, user_id) DO NOTHING;

        UPDATE parent_invitations
           SET status = 'accepted', accepted_at = now()
         WHERE id = r.id;

        v_count := v_count + 1;
    END LOOP;

    RETURN v_count;
END;
$$;

REVOKE ALL ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) TO aulalite_app;
