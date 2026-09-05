-- migrations/20260529000025_tenant_invitations.sql
-- Tenant member-invite data layer + cross-tenant JIT acceptance.
--
-- A `tenant_invitation` is an org-admin-issued invite for an email to become a
-- member (seat) of the tenant under a given role. It becomes (or re-activates)
-- a `tenant_memberships` row when the invited email first signs in (JIT
-- acceptance).
--
-- This mirrors `parent_invitations` (migration 20260529000023_parent_links.sql):
--   * a partial-unique on (tenant_id, lower(email)) WHERE status='pending' makes
--     create_invitation idempotent for a repeated pending invite;
--   * strict tenant_isolation RLS + FORCE so the non-bypass `aulalite_app` role
--     only ever sees its own tenant's rows for staff-facing reads/writes;
--   * a SECURITY DEFINER acceptance fn for the cross-tenant write that runs at
--     JIT-provisioning time, where there is NO per-request tenant GUC and one
--     email may have pending invites across SEVERAL tenants.
--
-- SEAT NOTE: seat-cap enforcement is done at invitation-CREATION time in Rust
-- (db::seats::seat_usage + handlers/admin.rs + handlers/member_invitations.rs);
-- accept_tenant_invitations_for_email itself does NOT do any seat math, exactly
-- like accept_parent_invitations_for_email. Acceptance always (re)activates the
-- membership for an already-issued invite.

-- ---------------------------------------------------------------------------
-- tenant_invitations
-- ---------------------------------------------------------------------------
CREATE TABLE tenant_invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    email CITEXT NOT NULL,
    role TEXT NOT NULL
        CHECK (role IN ('org_admin','teacher','ta','student','parent')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','accepted','revoked')),
    created_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at TIMESTAMPTZ
);

-- At most one PENDING invite per (tenant, email). Mirrors the partial-unique on
-- parent_invitations and lets create_invitation be idempotent.
CREATE UNIQUE INDEX tenant_invitations_pending_unique
    ON tenant_invitations (tenant_id, lower(email))
    WHERE status = 'pending';

CREATE INDEX tenant_invitations_tenant_idx ON tenant_invitations (tenant_id);

ALTER TABLE tenant_invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_invitations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON tenant_invitations
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON tenant_invitations TO aulalite_app;

-- ---------------------------------------------------------------------------
-- Cross-tenant acceptance helper.
--
-- Runs at JIT-provisioning time, where the invited user has just been created
-- and there is no per-request tenant context. For EVERY pending invitation
-- matching the signed-in email (across ALL tenants) it:
--   1. upserts an active tenant_memberships row with the invite's role — on a
--      pre-existing membership it (re)activates and re-roles it (so an invite
--      can re-activate a suspended member or change their role),
--   2. flips the invitation to 'accepted'.
-- Returns the number of invitations accepted.
--
-- It does NOT do seat math: seat-cap enforcement happens at CREATION time in
-- Rust (see the SEAT NOTE above), mirroring accept_parent_invitations_for_email.
--
-- SECURITY DEFINER so the writes run with the table-owning migration role's
-- privileges and bypass the strict tenant_isolation policies (we can't set a
-- single app.tenant_id when several tenants are involved). This mirrors the
-- existing SECURITY DEFINER acceptance/lookup helpers; it is the codebase's
-- sanctioned way to do cross-tenant work that precedes per-request tenant
-- resolution.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION accept_tenant_invitations_for_email(
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
        SELECT id, tenant_id, role
          FROM tenant_invitations
         WHERE status = 'pending'
           AND lower(email::text) = lower(p_email)
    LOOP
        INSERT INTO tenant_memberships
            (tenant_id, user_id, role, status)
        VALUES (r.tenant_id, p_user_id, r.role, 'active')
        ON CONFLICT (tenant_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                status = 'active';

        UPDATE tenant_invitations
           SET status = 'accepted', accepted_at = now()
         WHERE id = r.id;

        v_count := v_count + 1;
    END LOOP;

    RETURN v_count;
END;
$$;

REVOKE ALL ON FUNCTION accept_tenant_invitations_for_email(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_tenant_invitations_for_email(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION accept_tenant_invitations_for_email(UUID, TEXT) TO aulalite_app;
