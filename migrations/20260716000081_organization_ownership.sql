-- A tenant has one transferable organization owner above its administrators.
-- Ordinary member administration may never create or remove ownership; only
-- initial platform provisioning, personal-workspace provisioning, the audited
-- transfer endpoint, and the explicit platform recovery function do so.

ALTER TABLE tenant_memberships
    DROP CONSTRAINT IF EXISTS tenant_memberships_role_check;
ALTER TABLE tenant_memberships
    ADD CONSTRAINT tenant_memberships_role_check
    CHECK (role IN ('org_owner','org_admin','teacher','ta','student','parent'));

ALTER TABLE tenant_invitations
    DROP CONSTRAINT IF EXISTS tenant_invitations_role_check;
ALTER TABLE tenant_invitations
    ADD CONSTRAINT tenant_invitations_role_check
    CHECK (role IN ('org_owner','org_admin','teacher','ta','student','parent'));

-- The marker makes the rollout safe for any historical, empty tenant that has
-- neither a member nor an invitation from which ownership can be recovered.
-- Every supported production provisioning path initializes it atomically.
ALTER TABLE tenants ADD COLUMN ownership_initialized_at TIMESTAMPTZ;
ALTER TABLE tenants ALTER COLUMN ownership_initialized_at SET DEFAULT now();

-- Backfill the oldest active administrator. If a freshly provisioned tenant
-- has not yet been claimed, convert only its oldest pending initial-admin
-- invitation. A pending owner invite is the unavoidable provisional state
-- before that identity signs in and becomes the active owner membership.
WITH ranked_admins AS (
    SELECT tm.tenant_id,
           tm.user_id,
           row_number() OVER (
               PARTITION BY tm.tenant_id
               ORDER BY tm.joined_at, tm.user_id
           ) AS position
      FROM tenant_memberships tm
     WHERE tm.role = 'org_admin' AND tm.status = 'active'
)
UPDATE tenant_memberships tm
   SET role = 'org_owner', updated_at = now()
  FROM ranked_admins ranked
 WHERE tm.tenant_id = ranked.tenant_id
   AND tm.user_id = ranked.user_id
   AND ranked.position = 1;

WITH tenants_without_owner AS (
    SELECT t.id
      FROM tenants t
     WHERE NOT EXISTS (
        SELECT 1 FROM tenant_memberships tm
         WHERE tm.tenant_id = t.id
           AND tm.role = 'org_owner'
           AND tm.status = 'active'
     )
), ranked_invitations AS (
    SELECT ti.id,
           row_number() OVER (
               PARTITION BY ti.tenant_id
               ORDER BY ti.created_at, ti.id
           ) AS position
      FROM tenant_invitations ti
      JOIN tenants_without_owner missing ON missing.id = ti.tenant_id
     WHERE ti.role = 'org_admin'
       AND ti.status = 'pending'
       -- Only a platform-created initial-admin invitation is authoritative
       -- enough to become ownership during migration. Ordinary historical
       -- admin invites remain unchanged and their tenant stays recoverable as
       -- legacy/uninitialized.
       AND EXISTS (
            SELECT 1
              FROM users creator
             WHERE creator.id = ti.created_by
               AND creator.is_platform_admin
               AND creator.deleted_at IS NULL
               AND creator.identity_kind = 'global'
               AND creator.identity_tenant_id IS NULL
       )
)
UPDATE tenant_invitations ti
   SET role = 'org_owner'
  FROM ranked_invitations ranked
 WHERE ti.id = ranked.id AND ranked.position = 1;

UPDATE tenants t
   SET ownership_initialized_at = now()
 WHERE EXISTS (
        SELECT 1 FROM tenant_memberships tm
         WHERE tm.tenant_id = t.id
           AND tm.role = 'org_owner'
           AND tm.status = 'active'
    )
    OR EXISTS (
        SELECT 1 FROM tenant_invitations ti
         WHERE ti.tenant_id = t.id
           AND ti.role = 'org_owner'
           AND ti.status = 'pending'
    );

-- Historical administrators could previously issue peer-admin invitations.
-- After ownership is introduced, only the newly selected owner (or a live
-- platform operator) may remain the authority behind a pending admin invite.
-- Revoke stale invitations now so they cannot appoint an administrator after
-- the new hierarchy is active.
UPDATE tenant_invitations ti
   SET status = 'revoked'
 WHERE ti.role = 'org_admin'
   AND ti.status = 'pending'
   AND NOT EXISTS (
        SELECT 1 FROM tenant_memberships owner_membership
         WHERE owner_membership.tenant_id = ti.tenant_id
           AND owner_membership.user_id = ti.created_by
           AND owner_membership.role = 'org_owner'
           AND owner_membership.status = 'active'
   )
   AND NOT EXISTS (
        SELECT 1 FROM users platform_actor
         WHERE platform_actor.id = ti.created_by
           AND platform_actor.is_platform_admin
           AND platform_actor.deleted_at IS NULL
           AND platform_actor.identity_kind = 'global'
           AND platform_actor.identity_tenant_id IS NULL
   );

CREATE UNIQUE INDEX tenant_memberships_one_org_owner
    ON tenant_memberships (tenant_id)
    WHERE role = 'org_owner';

CREATE UNIQUE INDEX tenant_invitations_one_pending_org_owner
    ON tenant_invitations (tenant_id)
    WHERE role = 'org_owner' AND status = 'pending';

-- Harden the three legacy capability-token readers against temporary-table
-- shadowing. When pg_temp is omitted PostgreSQL searches it before named
-- schemas for relations, even inside a fixed search_path. Keeping it explicit
-- and last ensures the table owner's public relations always win.
ALTER FUNCTION lookup_enrollment_code(TEXT)
    SET search_path = public, pg_temp;
ALTER FUNCTION lookup_invitation_by_token(TEXT)
    SET search_path = public, pg_temp;
ALTER FUNCTION verify_certificate(TEXT)
    SET search_path = public, pg_temp;

-- Workspace resolution happens before the request has a selected tenant, but
-- it still has an authenticated global user. Do not let the granted app role
-- supply a different user UUID and enumerate that user's tenant memberships.
CREATE OR REPLACE FUNCTION resolve_active_workspace(
    p_user_id UUID,
    p_requested_tenant_id UUID DEFAULT NULL
)
RETURNS TABLE (tenant_id UUID, role TEXT)
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
BEGIN
    IF p_user_id IS NULL OR v_actor_user_id IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'authenticated_actor_mismatch' USING ERRCODE = '42501';
    END IF;

    RETURN QUERY
    SELECT tm.tenant_id, tm.role
      FROM tenant_memberships tm
     WHERE tm.user_id = p_user_id
       AND tm.status = 'active'
       AND (p_requested_tenant_id IS NULL OR tm.tenant_id = p_requested_tenant_id)
       AND tenant_access_allowed(tm.tenant_id)
     ORDER BY tm.joined_at, tm.tenant_id
     LIMIT 1;
END;
$$;

REVOKE ALL ON FUNCTION resolve_active_workspace(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_active_workspace(UUID, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION resolve_active_workspace(UUID, UUID) TO aulalite_app;

-- Parent invitation acceptance is a global-identity JIT operation. Bind both
-- caller-controlled arguments to the live identity and authenticated actor
-- before the definer performs any cross-tenant membership or parent-link write.
CREATE OR REPLACE FUNCTION accept_parent_invitations_for_email_v2(
        p_user_id UUID,
        p_email TEXT
    )
    RETURNS TABLE(accepted_count INTEGER, blocked_tenant_count INTEGER)
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    r_tenant RECORD;
    v_status TEXT;
    v_existing_role TEXT;
    v_active BIGINT;
    v_pending BIGINT;
    v_cap BIGINT;
    v_overage TEXT;
    v_has_reserved_seat BOOLEAN;
    v_rows INTEGER;
BEGIN
    IF p_user_id IS NULL
       OR v_actor_user_id IS DISTINCT FROM p_user_id
       OR NOT EXISTS (
            SELECT 1 FROM users u
             WHERE u.id = p_user_id
               AND u.deleted_at IS NULL
               AND u.identity_kind = 'global'
               AND u.identity_tenant_id IS NULL
               AND lower(u.email::text) = lower(p_email)
       ) THEN
        RAISE EXCEPTION 'invitation_identity_mismatch' USING ERRCODE = '42501';
    END IF;

    accepted_count := 0;
    blocked_tenant_count := 0;

    -- Lock all affected tenants in a stable order before touching memberships.
    PERFORM t.id
      FROM tenants t
     WHERE t.id IN (
        SELECT DISTINCT pi.tenant_id
          FROM parent_invitations pi
         WHERE pi.status = 'pending'
           AND lower(pi.parent_email::text) = lower(p_email)
     )
     ORDER BY t.id
     FOR UPDATE;

    FOR r_tenant IN
        SELECT DISTINCT pi.tenant_id
          FROM parent_invitations pi
         WHERE pi.status = 'pending'
           AND lower(pi.parent_email::text) = lower(p_email)
         ORDER BY pi.tenant_id
    LOOP
        v_status := NULL;
        v_existing_role := NULL;
        SELECT tm.status, tm.role
          INTO v_status, v_existing_role
          FROM tenant_memberships tm
         WHERE tm.tenant_id = r_tenant.tenant_id
           AND tm.user_id = p_user_id
         FOR UPDATE;

        -- Suspension is an explicit administrator decision. A stale invitation
        -- must never turn that membership active again during sign-in.
        IF v_status = 'suspended' THEN
            UPDATE parent_invitations pi
               SET status = 'revoked'
             WHERE pi.tenant_id = r_tenant.tenant_id
               AND pi.status = 'pending'
               AND lower(pi.parent_email::text) = lower(p_email);
            CONTINUE;
        END IF;

        -- A parent invitation must never reactivate, demote, or otherwise
        -- repurpose an existing organization authority membership. The single
        -- tenant-role model requires that conflict to be resolved explicitly.
        IF v_existing_role IN ('org_owner', 'org_admin') THEN
            UPDATE parent_invitations pi
               SET status = 'revoked'
             WHERE pi.tenant_id = r_tenant.tenant_id
               AND pi.status = 'pending'
               AND lower(pi.parent_email::text) = lower(p_email);
            CONTINUE;
        END IF;

        -- Existing active members consume no additional seat and remain
        -- idempotently eligible even if the tenant is already over cap.
        IF v_status IS DISTINCT FROM 'active' THEN
            SELECT
                (SELECT COUNT(*)
                   FROM tenant_memberships tm
                  WHERE tm.tenant_id = r_tenant.tenant_id
                    AND tm.status = 'active'),
                (SELECT COUNT(*)
                   FROM (
                       SELECT lower(ti.email::text) AS email_key
                         FROM tenant_invitations ti
                        WHERE ti.tenant_id = r_tenant.tenant_id
                          AND ti.status = 'pending'
                       UNION
                       SELECT lower(pi.parent_email::text) AS email_key
                         FROM parent_invitations pi
                        WHERE pi.tenant_id = r_tenant.tenant_id
                          AND pi.status = 'pending'
                          AND pi.seat_reserved
                   ) reserved),
                (SELECT p.included_seats::bigint
                   FROM subscriptions s
                   LEFT JOIN plans p ON p.id = s.plan_id
                  WHERE s.tenant_id = r_tenant.tenant_id),
                COALESCE(
                    (SELECT s.overage_behavior
                       FROM subscriptions s
                      WHERE s.tenant_id = r_tenant.tenant_id),
                    'block'
                )
              INTO v_active, v_pending, v_cap, v_overage;

            -- A matching tenant member invitation already reserved this
            -- user's seat. The earlier member-invitation acceptance in the
            -- same JIT transaction converts that reservation to active.
            SELECT
                EXISTS (
                    SELECT 1
                      FROM tenant_invitations ti
                     WHERE ti.tenant_id = r_tenant.tenant_id
                       AND ti.status = 'pending'
                       AND lower(ti.email::text) = lower(p_email)
                ) OR EXISTS (
                    SELECT 1
                      FROM parent_invitations pi
                     WHERE pi.tenant_id = r_tenant.tenant_id
                       AND pi.status = 'pending'
                       AND pi.seat_reserved
                       AND lower(pi.parent_email::text) = lower(p_email)
                )
              INTO v_has_reserved_seat;

            IF NOT v_has_reserved_seat
               AND v_overage = 'block'
               AND v_cap IS NOT NULL
               AND v_active + v_pending >= v_cap THEN
                blocked_tenant_count := blocked_tenant_count + 1;
                CONTINUE;
            END IF;

            IF v_status IS NULL THEN
                INSERT INTO tenant_memberships
                    (tenant_id, user_id, role, status)
                VALUES (r_tenant.tenant_id, p_user_id, 'parent', 'active');
            ELSE
                UPDATE tenant_memberships
                   SET status = 'active', updated_at = now()
                 WHERE tenant_id = r_tenant.tenant_id
                   AND user_id = p_user_id;
            END IF;
        END IF;

        INSERT INTO parent_links
            (parent_user_id, student_user_id, tenant_id, relationship)
        SELECT p_user_id, pi.student_user_id, pi.tenant_id, pi.relationship
          FROM parent_invitations pi
         WHERE pi.tenant_id = r_tenant.tenant_id
           AND pi.status = 'pending'
           AND lower(pi.parent_email::text) = lower(p_email)
        ON CONFLICT DO NOTHING;

        UPDATE parent_invitations pi
           SET status = 'accepted', accepted_at = now()
         WHERE pi.tenant_id = r_tenant.tenant_id
           AND pi.status = 'pending'
           AND lower(pi.parent_email::text) = lower(p_email);
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        accepted_count := accepted_count + v_rows;
    END LOOP;

    RETURN NEXT;
END;
$$;

REVOKE ALL ON FUNCTION accept_parent_invitations_for_email_v2(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email_v2(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email_v2(UUID, TEXT) TO aulalite_app;

-- Keep the rolling-release compatibility entry point, but delegate to the
-- identity-bound v2 implementation above.
CREATE OR REPLACE FUNCTION accept_parent_invitations_for_email(
        p_user_id UUID,
        p_email TEXT
    )
    RETURNS INTEGER
    LANGUAGE sql
    SECURITY DEFINER
    SET search_path = public, pg_temp
AS $$
    SELECT accepted_count
      FROM accept_parent_invitations_for_email_v2(p_user_id, p_email)
$$;

REVOKE ALL ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION accept_parent_invitations_for_email(UUID, TEXT) TO aulalite_app;

-- Owner invitations are definer-only. Administrator invitations and mutations
-- require the active organization owner when performed by the runtime role.
-- The invoker check cannot be spoofed with `created_by`: nested DML from an
-- approved SECURITY DEFINER function runs as the table/function owner, while a
-- direct `aulalite_app` statement does not. No role name is hard-coded.
CREATE OR REPLACE FUNCTION authorize_org_owner_invitation()
RETURNS trigger
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_is_table_owner BOOLEAN;
    v_tenant_id UUID := CASE WHEN TG_OP = 'DELETE' THEN OLD.tenant_id ELSE NEW.tenant_id END;
    v_old_role TEXT := CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE OLD.role END;
    v_new_role TEXT := CASE WHEN TG_OP = 'DELETE' THEN NULL ELSE NEW.role END;
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
BEGIN
    SELECT actor_role.oid = relation.relowner
      INTO v_is_table_owner
      FROM pg_catalog.pg_roles actor_role
      JOIN pg_catalog.pg_class relation ON relation.oid = TG_RELID
     WHERE actor_role.rolname = current_user;

    IF NOT COALESCE(v_is_table_owner, FALSE) THEN
        IF v_old_role = 'org_owner' OR v_new_role = 'org_owner' THEN
            RAISE EXCEPTION 'org_owner_invitation_requires_definer'
                USING ERRCODE = '42501';
        END IF;
        IF (v_old_role = 'org_admin' OR v_new_role = 'org_admin')
           AND NOT EXISTS (
                SELECT 1 FROM tenant_memberships tm
                 WHERE tm.tenant_id = v_tenant_id
                   AND tm.user_id = v_actor_user_id
                   AND tm.role = 'org_owner'
                   AND tm.status = 'active'
           ) THEN
            RAISE EXCEPTION 'organization_owner_required_for_admin_invitation'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    IF TG_OP <> 'DELETE' AND NEW.role = 'org_owner' AND NEW.status = 'pending' THEN
        -- Serialize initial/recovery owner invitations with ownership transfer
        -- and acceptance, all of which use this same tenant-row lock.
        PERFORM id FROM tenants WHERE id = NEW.tenant_id FOR UPDATE;
    END IF;
    IF TG_OP <> 'DELETE' AND NEW.role = 'org_owner'
       AND (TG_OP = 'INSERT'
            OR OLD.role <> 'org_owner'
            OR NEW.created_by IS DISTINCT FROM OLD.created_by) THEN
        IF NEW.created_by IS NULL OR NOT EXISTS (
            SELECT 1 FROM users u
             WHERE u.id = NEW.created_by
               AND u.is_platform_admin
               AND u.deleted_at IS NULL
               AND u.identity_kind = 'global'
               AND u.identity_tenant_id IS NULL
        ) THEN
            RAISE EXCEPTION 'org_owner_invitation_requires_platform_admin'
                USING ERRCODE = '42501';
        END IF;
    END IF;
    IF TG_OP <> 'DELETE'
       AND NEW.role = 'org_owner'
       AND NEW.status = 'pending'
       AND EXISTS (
        SELECT 1 FROM tenant_memberships tm
         WHERE tm.tenant_id = NEW.tenant_id
           AND tm.role = 'org_owner'
           AND tm.status = 'active'
    ) THEN
        RAISE EXCEPTION 'tenant_already_has_active_org_owner'
            USING ERRCODE = '23505',
                  CONSTRAINT = 'tenant_invitations_one_pending_org_owner';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenant_invitations_authorize_owner
BEFORE INSERT OR UPDATE OR DELETE ON tenant_invitations
FOR EACH ROW
EXECUTE FUNCTION authorize_org_owner_invitation();

REVOKE ALL ON FUNCTION authorize_org_owner_invitation() FROM PUBLIC;

-- The cross-tenant global-identity acceptance helper is granted to the app DB
-- role, so it must not trust its caller's email parameter. Bind the invitation
-- claim to the stored, live global identity before acquiring tenant locks.
-- Enterprise identities keep their existing tenant-local acceptance path in
-- `db::sso::ensure_enterprise_tenant_membership` and cannot use this helper to
-- claim an invitation in another organization.
CREATE OR REPLACE FUNCTION accept_tenant_invitations_for_email(
        p_user_id UUID,
        p_email TEXT
    )
    RETURNS INTEGER
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_count INTEGER := 0;
    v_status TEXT;
    v_existing_role TEXT;
    r RECORD;
BEGIN
    IF p_user_id IS NULL
       OR v_actor_user_id IS DISTINCT FROM p_user_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_user_id
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
           AND lower(u.email::text) = lower(p_email)
       ) THEN
        RAISE EXCEPTION 'invitation_identity_mismatch' USING ERRCODE = '42501';
    END IF;

    PERFORM t.id
      FROM tenants t
     WHERE t.id IN (
        SELECT DISTINCT ti.tenant_id
          FROM tenant_invitations ti
         WHERE ti.status = 'pending'
           AND lower(ti.email::text) = lower(p_email)
     )
     ORDER BY t.id
     FOR UPDATE;

    FOR r IN
        SELECT id, tenant_id, role
          FROM tenant_invitations
         WHERE status = 'pending'
           AND lower(email::text) = lower(p_email)
         ORDER BY tenant_id, id
    LOOP
        v_status := NULL;
        v_existing_role := NULL;
        SELECT tm.status, tm.role INTO v_status, v_existing_role
          FROM tenant_memberships tm
         WHERE tm.tenant_id = r.tenant_id
           AND tm.user_id = p_user_id
         FOR UPDATE;

        IF v_status = 'suspended' THEN
            UPDATE tenant_invitations SET status = 'revoked' WHERE id = r.id;
            CONTINUE;
        END IF;

        -- Invitation acceptance is not a general membership-role editor. A
        -- lower-role invite must never demote an existing organization owner
        -- or administrator. The sole allowed privileged transition is an
        -- administrator claiming a platform-authored owner invitation.
        IF (v_existing_role = 'org_owner' AND r.role <> 'org_owner')
           OR (v_existing_role = 'org_admin'
               AND r.role NOT IN ('org_owner', 'org_admin')) THEN
            UPDATE tenant_invitations SET status = 'revoked' WHERE id = r.id;
            CONTINUE;
        END IF;

        INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
        VALUES (r.tenant_id, p_user_id, r.role, 'active')
        ON CONFLICT (tenant_id, user_id) DO UPDATE
            SET role = EXCLUDED.role,
                status = 'active',
                updated_at = now();

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

-- Enterprise identities accept only a concrete invitation in their asserted
-- tenant. Privileged invitation consumption runs under the table owner so the
-- role-authorization triggers cannot be bypassed by ordinary runtime SQL, while
-- all identity/tenant/email predicates are revalidated inside this boundary.
CREATE OR REPLACE FUNCTION accept_privileged_enterprise_invitation(
    p_invitation_id UUID,
    p_tenant_id UUID,
    p_user_id UUID,
    p_asserted_email TEXT
)
RETURNS TEXT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_actor_tenant_id UUID := NULLIF(current_setting('app.tenant_id', true), '')::uuid;
    v_role TEXT;
    v_status TEXT;
    v_existing_role TEXT;
BEGIN
    IF p_user_id IS NULL
       OR p_tenant_id IS NULL
       OR v_actor_user_id IS DISTINCT FROM p_user_id
       OR v_actor_tenant_id IS DISTINCT FROM p_tenant_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_user_id
           AND u.deleted_at IS NULL
           AND u.identity_kind IN ('sso', 'lti')
           AND u.identity_tenant_id = p_tenant_id
           AND lower(u.email::text) = lower(p_asserted_email)
       ) THEN
        RAISE EXCEPTION 'enterprise_invitation_identity_mismatch'
            USING ERRCODE = '42501';
    END IF;

    PERFORM id FROM tenants WHERE id = p_tenant_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tenant_not_found' USING ERRCODE = 'P0002';
    END IF;

    SELECT ti.role INTO v_role
      FROM tenant_invitations ti
     WHERE ti.id = p_invitation_id
       AND ti.tenant_id = p_tenant_id
       AND ti.status = 'pending'
       AND ti.role IN ('org_owner', 'org_admin')
       AND lower(ti.email::text) = lower(p_asserted_email)
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'privileged_invitation_not_found' USING ERRCODE = 'P0002';
    END IF;

    SELECT tm.status, tm.role INTO v_status, v_existing_role
      FROM tenant_memberships tm
     WHERE tm.tenant_id = p_tenant_id AND tm.user_id = p_user_id
     FOR UPDATE;
    IF v_status = 'suspended' THEN
        UPDATE tenant_invitations SET status = 'revoked'
         WHERE id = p_invitation_id;
        RETURN 'suspended';
    END IF;

    -- Preserve existing organization authority. An administrator may claim a
    -- platform-authored owner invitation, but an admin invite can never demote
    -- the active owner.
    IF v_existing_role = 'org_owner' AND v_role <> 'org_owner' THEN
        UPDATE tenant_invitations SET status = 'revoked'
         WHERE id = p_invitation_id;
        RETURN 'preserved';
    END IF;

    INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
    VALUES (p_tenant_id, p_user_id, v_role, 'active')
    ON CONFLICT (tenant_id, user_id) DO UPDATE
        SET role = EXCLUDED.role, status = 'active', updated_at = now();
    UPDATE tenant_invitations
       SET status = 'accepted', accepted_at = now()
     WHERE id = p_invitation_id;
    RETURN 'activated';
END;
$$;

REVOKE ALL ON FUNCTION accept_privileged_enterprise_invitation(UUID, UUID, UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_privileged_enterprise_invitation(UUID, UUID, UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION accept_privileged_enterprise_invitation(UUID, UUID, UUID, TEXT) TO aulalite_app;

-- Replace the last-administrator rule. Admins are delegated operators and may
-- be absent; ownership is the tenant recovery and billing authority.
DROP TRIGGER IF EXISTS tenant_memberships_protect_last_admin ON tenant_memberships;
DROP FUNCTION IF EXISTS protect_last_active_org_admin();

CREATE OR REPLACE FUNCTION authorize_privileged_membership_mutation()
RETURNS trigger
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_is_table_owner BOOLEAN;
    v_tenant_id UUID := CASE WHEN TG_OP = 'DELETE' THEN OLD.tenant_id ELSE NEW.tenant_id END;
    v_old_role TEXT := CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE OLD.role END;
    v_new_role TEXT := CASE WHEN TG_OP = 'DELETE' THEN NULL ELSE NEW.role END;
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
BEGIN
    SELECT actor_role.oid = relation.relowner
      INTO v_is_table_owner
      FROM pg_catalog.pg_roles actor_role
      JOIN pg_catalog.pg_class relation ON relation.oid = TG_RELID
     WHERE actor_role.rolname = current_user;
    IF COALESCE(v_is_table_owner, FALSE) THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;

    IF v_old_role = 'org_owner' OR v_new_role = 'org_owner' THEN
        RAISE EXCEPTION 'org_owner_membership_requires_definer'
            USING ERRCODE = '42501';
    END IF;
    IF (v_old_role = 'org_admin' OR v_new_role = 'org_admin')
       AND NOT EXISTS (
            SELECT 1 FROM tenant_memberships tm
             WHERE tm.tenant_id = v_tenant_id
               AND tm.user_id = v_actor_user_id
               AND tm.role = 'org_owner'
               AND tm.status = 'active'
       ) THEN
        RAISE EXCEPTION 'organization_owner_required_for_admin_membership'
            USING ERRCODE = '42501';
    END IF;

    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenant_memberships_authorize_privileged_role
BEFORE INSERT OR UPDATE OR DELETE ON tenant_memberships
FOR EACH ROW
EXECUTE FUNCTION authorize_privileged_membership_mutation();

REVOKE ALL ON FUNCTION authorize_privileged_membership_mutation() FROM PUBLIC;

-- A tenant-scoped membership or invitation is never reassigned in place.
-- Rejecting scope changes up front ensures the deferred owner invariant checks
-- cannot validate only NEW.tenant_id while silently leaving OLD.tenant_id
-- ownerless. Legitimate moves must be explicit delete/insert operations, which
-- cause both tenant invariants to run.
CREATE OR REPLACE FUNCTION reject_tenant_scope_reassignment()
RETURNS trigger
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id THEN
        RAISE EXCEPTION 'tenant_scope_reassignment_forbidden'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenant_memberships_reject_tenant_reassignment
BEFORE UPDATE OF tenant_id ON tenant_memberships
FOR EACH ROW
EXECUTE FUNCTION reject_tenant_scope_reassignment();

CREATE TRIGGER tenant_invitations_reject_tenant_reassignment
BEFORE UPDATE OF tenant_id ON tenant_invitations
FOR EACH ROW
EXECUTE FUNCTION reject_tenant_scope_reassignment();

REVOKE ALL ON FUNCTION reject_tenant_scope_reassignment() FROM PUBLIC;

CREATE OR REPLACE FUNCTION protect_tenant_ownership_marker()
RETURNS trigger
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_is_table_owner BOOLEAN;
BEGIN
    SELECT actor_role.oid = relation.relowner
      INTO v_is_table_owner
      FROM pg_catalog.pg_roles actor_role
      JOIN pg_catalog.pg_class relation ON relation.oid = TG_RELID
     WHERE actor_role.rolname = current_user;

    IF NOT COALESCE(v_is_table_owner, FALSE) THEN
        IF TG_OP = 'INSERT' AND NEW.ownership_initialized_at IS NULL THEN
            RAISE EXCEPTION 'ownership_initialization_required'
                USING ERRCODE = '42501';
        END IF;
        IF TG_OP = 'UPDATE'
           AND NEW.ownership_initialized_at IS DISTINCT FROM OLD.ownership_initialized_at THEN
            RAISE EXCEPTION 'ownership_marker_is_definer_managed'
                USING ERRCODE = '42501';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER tenants_protect_ownership_marker
BEFORE INSERT OR UPDATE OF ownership_initialized_at ON tenants
FOR EACH ROW
EXECUTE FUNCTION protect_tenant_ownership_marker();

REVOKE ALL ON FUNCTION protect_tenant_ownership_marker() FROM PUBLIC;

CREATE OR REPLACE FUNCTION enforce_tenant_owner_invariant()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_tenant_id UUID;
    v_owner_rows BIGINT;
    v_active_owners BIGINT;
    v_pending_owner_invitations BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'tenants' THEN
        v_tenant_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    ELSE
        v_tenant_id := CASE
            WHEN TG_OP = 'DELETE' THEN OLD.tenant_id
            ELSE NEW.tenant_id
        END;
    END IF;

    -- Tenant deletion intentionally cascades through memberships.
    IF NOT EXISTS (SELECT 1 FROM tenants WHERE id = v_tenant_id) THEN
        RETURN NULL;
    END IF;

    -- A historical empty shell cannot be assigned an owner automatically.
    -- It remains explicitly uninitialized until platform recovery selects an
    -- active administrator; all normal production-created tenants are marked.
    IF NOT EXISTS (
        SELECT 1 FROM tenants
         WHERE id = v_tenant_id AND ownership_initialized_at IS NOT NULL
    ) THEN
        RETURN NULL;
    END IF;

    SELECT count(*), count(*) FILTER (WHERE tm.status = 'active')
      INTO v_owner_rows, v_active_owners
      FROM tenant_memberships tm
     WHERE tm.tenant_id = v_tenant_id
       AND tm.role = 'org_owner';

    SELECT count(*) INTO v_pending_owner_invitations
      FROM tenant_invitations ti
     WHERE ti.tenant_id = v_tenant_id
       AND ti.role = 'org_owner'
       AND ti.status = 'pending';

    IF v_owner_rows = 1
       AND v_active_owners = 1
       AND v_pending_owner_invitations = 0 THEN
        RETURN NULL;
    END IF;

    -- Platform-created workspaces temporarily have a pending, privileged owner
    -- invitation until the verified recipient signs in. No other ownerless
    -- state is valid at transaction end.
    IF v_owner_rows = 0 AND v_pending_owner_invitations = 1 THEN
        RETURN NULL;
    END IF;

    RAISE EXCEPTION 'active_org_owner_required'
        USING ERRCODE = '23514',
              CONSTRAINT = 'tenant_memberships_active_org_owner_required';
END;
$$;

CREATE CONSTRAINT TRIGGER tenant_memberships_require_active_owner
AFTER INSERT OR UPDATE OR DELETE ON tenant_memberships
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION enforce_tenant_owner_invariant();

CREATE CONSTRAINT TRIGGER tenant_invitations_require_active_owner
AFTER INSERT OR UPDATE OR DELETE ON tenant_invitations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION enforce_tenant_owner_invariant();

CREATE CONSTRAINT TRIGGER tenants_require_active_owner
AFTER INSERT OR UPDATE OR DELETE ON tenants
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION enforce_tenant_owner_invariant();

REVOKE ALL ON FUNCTION enforce_tenant_owner_invariant() FROM PUBLIC;

-- Ordinary ownership transfer is one audited, actor-validating database
-- operation. Runtime SQL cannot directly mutate an org_owner row because the
-- invoker trigger above permits those changes only under this definer.
CREATE OR REPLACE FUNCTION transfer_tenant_ownership(
    p_tenant_id UUID,
    p_target_user_id UUID
)
RETURNS TABLE (
    previous_owner_user_id UUID,
    new_owner_user_id UUID,
    transferred_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_transferred_at TIMESTAMPTZ := now();
BEGIN
    IF v_actor_user_id IS NULL THEN
        RAISE EXCEPTION 'authenticated_actor_required' USING ERRCODE = '42501';
    END IF;
    IF p_target_user_id = v_actor_user_id THEN
        RAISE EXCEPTION 'new_owner_must_be_another_admin' USING ERRCODE = '23514';
    END IF;

    PERFORM id FROM tenants WHERE id = p_tenant_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tenant_not_found' USING ERRCODE = 'P0002';
    END IF;
    PERFORM 1 FROM tenant_memberships
     WHERE tenant_id = p_tenant_id
       AND user_id = v_actor_user_id
       AND role = 'org_owner'
       AND status = 'active'
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'organization_owner_required' USING ERRCODE = '42501';
    END IF;
    PERFORM 1
      FROM tenant_memberships tm
      JOIN users u ON u.id = tm.user_id AND u.deleted_at IS NULL
     WHERE tm.tenant_id = p_tenant_id
       AND tm.user_id = p_target_user_id
       AND tm.role = 'org_admin'
       AND tm.status = 'active'
     FOR UPDATE OF tm;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'target_must_be_active_org_admin' USING ERRCODE = '23514';
    END IF;

    UPDATE tenant_memberships
       SET role = 'org_admin', updated_at = v_transferred_at
     WHERE tenant_id = p_tenant_id AND user_id = v_actor_user_id;
    UPDATE tenant_memberships
       SET role = 'org_owner', updated_at = v_transferred_at
     WHERE tenant_id = p_tenant_id AND user_id = p_target_user_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        p_tenant_id, v_actor_user_id, 'tenant.ownership.transferred',
        'tenant', p_tenant_id,
        jsonb_build_object(
            'previous_owner_user_id', v_actor_user_id,
            'new_owner_user_id', p_target_user_id
        )
    );

    RETURN QUERY SELECT v_actor_user_id, p_target_user_id, v_transferred_at;
END;
$$;

REVOKE ALL ON FUNCTION transfer_tenant_ownership(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION transfer_tenant_ownership(UUID, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION transfer_tenant_ownership(UUID, UUID) TO aulalite_app;

-- Self-service verified users own their automatically provisioned workspace.
CREATE OR REPLACE FUNCTION provision_personal_workspace_if_needed(
        p_user_id UUID,
        p_workspace_name TEXT
    )
    RETURNS UUID
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_tenant_id UUID;
    v_workspace_name TEXT;
BEGIN
    IF p_user_id IS NULL OR v_actor_user_id IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'authenticated_actor_mismatch' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM users u
         WHERE u.id = p_user_id
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
           AND u.deleted_at IS NULL
    ) THEN
        RAISE EXCEPTION 'live_global_identity_required' USING ERRCODE = '42501';
    END IF;

    PERFORM pg_advisory_xact_lock(
        hashtextextended('aulalite:personal-workspace:' || p_user_id::text, 0)
    );

    SELECT tm.tenant_id INTO v_tenant_id
      FROM tenant_memberships tm
     WHERE tm.user_id = p_user_id
     ORDER BY tm.joined_at, tm.tenant_id
     LIMIT 1;
    IF v_tenant_id IS NOT NULL THEN
        RETURN NULL;
    END IF;

    SELECT t.id INTO v_tenant_id
      FROM tenants t
     WHERE t.personal_owner_user_id = p_user_id;

    v_workspace_name := LEFT(
        COALESCE(NULLIF(BTRIM(p_workspace_name), ''), 'My academy'), 160
    );

    IF v_tenant_id IS NULL THEN
        v_tenant_id := gen_random_uuid();
        INSERT INTO tenants (id, slug, name, status, personal_owner_user_id)
        VALUES (
            v_tenant_id,
            'academy-' || REPLACE(p_user_id::text, '-', ''),
            v_workspace_name,
            'trialing',
            p_user_id
        );
        UPDATE tenants SET trial_ends_at = now() + interval '14 days'
         WHERE id = v_tenant_id;
        INSERT INTO subscriptions (
            tenant_id, plan_id, status, trial_ends_at, overage_behavior
        ) VALUES (
            v_tenant_id, 'starter', 'trialing', now() + interval '14 days', 'block'
        );
    END IF;

    INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
    VALUES (v_tenant_id, p_user_id, 'org_owner', 'active')
    ON CONFLICT (tenant_id, user_id) DO UPDATE
        SET role = 'org_owner', status = 'active', updated_at = now();
    UPDATE tenants
       SET ownership_initialized_at = COALESCE(ownership_initialized_at, now())
     WHERE id = v_tenant_id;

    RETURN v_tenant_id;
END;
$$;

REVOKE ALL ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) TO aulalite_app;

-- Platform tenant creation issues the sole privileged initial-owner invite.
-- Retire the pre-owner helper so no granted SQL path can create an ownerless
-- production workspace after this migration.
REVOKE ALL ON FUNCTION platform_create_tenant(TEXT, TEXT, TEXT) FROM aulalite;
REVOKE ALL ON FUNCTION platform_create_tenant(TEXT, TEXT, TEXT) FROM aulalite_app;
DROP FUNCTION platform_create_tenant(TEXT, TEXT, TEXT);

-- Re-secure the original platform directory/status helpers. Their legacy
-- definitions trusted the Rust handler alone, which left the granted app role
-- able to invoke the SECURITY DEFINER functions directly. Every cross-tenant
-- helper now authenticates the transaction-local platform actor itself.
CREATE OR REPLACE FUNCTION platform_list_tenants()
RETURNS TABLE (
    id UUID,
    slug TEXT,
    name TEXT,
    status TEXT,
    created_at TIMESTAMPTZ,
    member_count BIGINT,
    plan_id TEXT
)
LANGUAGE plpgsql
SECURITY DEFINER
STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = v_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;

    RETURN QUERY
    SELECT t.id,
           t.slug,
           t.name,
           t.status,
           t.created_at,
           COALESCE(m.member_count, 0),
           s.plan_id
      FROM tenants t
      LEFT JOIN (
            SELECT tm.tenant_id, COUNT(*) AS member_count
              FROM tenant_memberships tm
             WHERE tm.status = 'active'
             GROUP BY tm.tenant_id
      ) m ON m.tenant_id = t.id
      LEFT JOIN subscriptions s ON s.tenant_id = t.id
     ORDER BY t.created_at DESC;
END;
$$;

CREATE OR REPLACE FUNCTION platform_get_tenant_summary(p_id UUID)
RETURNS TABLE (
    id UUID,
    slug TEXT,
    name TEXT,
    status TEXT,
    created_at TIMESTAMPTZ,
    member_count BIGINT,
    plan_id TEXT
)
LANGUAGE plpgsql
SECURITY DEFINER
STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = v_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;

    RETURN QUERY
    SELECT t.id,
           t.slug,
           t.name,
           t.status,
           t.created_at,
           COALESCE(m.member_count, 0),
           s.plan_id
      FROM tenants t
      LEFT JOIN (
            SELECT tm.tenant_id, COUNT(*) AS member_count
              FROM tenant_memberships tm
             WHERE tm.status = 'active'
             GROUP BY tm.tenant_id
      ) m ON m.tenant_id = t.id
      LEFT JOIN subscriptions s ON s.tenant_id = t.id
     WHERE t.id = p_id;
END;
$$;

CREATE OR REPLACE FUNCTION platform_set_tenant_status(p_id UUID, p_status TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_previous_status TEXT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = v_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;
    IF p_status NOT IN ('active', 'trialing', 'suspended') THEN
        RAISE EXCEPTION 'invalid tenant status: %', p_status
            USING ERRCODE = 'check_violation';
    END IF;

    SELECT t.status
      INTO v_previous_status
      FROM tenants t
     WHERE t.id = p_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;

    UPDATE tenants
       SET status = p_status, updated_at = now()
     WHERE id = p_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        p_id, v_actor_user_id, 'platform.tenant.status', 'tenant', p_id,
        jsonb_build_object(
            'previous_status', v_previous_status,
            'status', p_status
        )
    );
    RETURN TRUE;
END;
$$;

REVOKE ALL ON FUNCTION platform_list_tenants() FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_get_tenant_summary(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION platform_set_tenant_status(UUID, TEXT) FROM PUBLIC;

CREATE OR REPLACE FUNCTION platform_provision_tenant(
    p_slug TEXT,
    p_name TEXT,
    p_admin_email TEXT,
    p_actor_user_id UUID
)
RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_authenticated_actor UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_tenant_id UUID;
    v_invitation_id UUID;
BEGIN
    IF p_actor_user_id IS NULL
       OR v_authenticated_actor IS DISTINCT FROM p_actor_user_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;
    IF p_admin_email IS NULL
       OR length(BTRIM(p_admin_email)) > 254
       OR BTRIM(p_admin_email) !~ '^[^[:space:]@]+@[^[:space:]@]+$' THEN
        RAISE EXCEPTION 'invalid_admin_email' USING ERRCODE = '23514';
    END IF;

    SELECT id INTO v_tenant_id FROM tenants WHERE slug = p_slug;
    IF v_tenant_id IS NOT NULL THEN
        IF EXISTS (
            SELECT 1 FROM tenant_invitations ti
             WHERE ti.tenant_id = v_tenant_id
               AND ti.status = 'pending'
               AND ti.role = 'org_owner'
               AND lower(ti.email::text) = lower(p_admin_email)
        ) THEN
            RETURN v_tenant_id;
        END IF;
        RAISE EXCEPTION 'tenant slug already exists: %', p_slug
            USING ERRCODE = 'unique_violation';
    END IF;

    INSERT INTO tenants (slug, name, status, trial_ends_at)
    VALUES (p_slug, p_name, 'trialing', now() + interval '14 days')
    RETURNING id INTO v_tenant_id;

    INSERT INTO subscriptions (
        tenant_id, plan_id, status, trial_ends_at, overage_behavior
    ) VALUES (
        v_tenant_id, 'starter', 'trialing', now() + interval '14 days', 'block'
    );

    INSERT INTO tenant_invitations (tenant_id, email, role, created_by)
    VALUES (v_tenant_id, p_admin_email, 'org_owner', p_actor_user_id)
    RETURNING id INTO v_invitation_id;

    UPDATE tenants SET ownership_initialized_at = now() WHERE id = v_tenant_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        v_tenant_id, p_actor_user_id, 'platform.tenant.create', 'tenant',
        v_tenant_id,
        jsonb_build_object(
            'slug', p_slug,
            'owner_email', p_admin_email,
            'invitation_id', v_invitation_id
        )
    );
    RETURN v_tenant_id;
END;
$$;

REVOKE ALL ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) TO aulalite_app;

-- Correct an initial owner-email typo without broadening ordinary invitation
-- authority. For a truly empty legacy/uninitialized shell, the same guarded
-- operation creates its first owner invitation. Both paths require that no
-- owner membership exists and finish in the one-pending-owner state.
CREATE OR REPLACE FUNCTION platform_replace_pending_owner_invitation(
    p_tenant_id UUID,
    p_new_email TEXT,
    p_actor_user_id UUID
)
RETURNS TABLE (
    invitation_id UUID,
    email TEXT,
    created_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_authenticated_actor UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_initialized BOOLEAN;
    v_invitation_id UUID;
    v_old_email TEXT;
    v_audit_action TEXT;
    v_created_at TIMESTAMPTZ := now();
BEGIN
    IF p_actor_user_id IS NULL
       OR v_authenticated_actor IS DISTINCT FROM p_actor_user_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;
    IF p_new_email IS NULL
       OR length(BTRIM(p_new_email)) > 254
       OR BTRIM(p_new_email) !~ '^[^[:space:]@]+@[^[:space:]@]+$' THEN
        RAISE EXCEPTION 'invalid_owner_email' USING ERRCODE = '23514';
    END IF;

    SELECT t.ownership_initialized_at IS NOT NULL
      INTO v_initialized
      FROM tenants t
     WHERE t.id = p_tenant_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tenant_not_found' USING ERRCODE = 'P0002';
    END IF;
    IF EXISTS (
        SELECT 1 FROM tenant_memberships tm
         WHERE tm.tenant_id = p_tenant_id AND tm.role = 'org_owner'
    ) THEN
        RAISE EXCEPTION 'organization_owner_already_claimed' USING ERRCODE = '23514';
    END IF;

    SELECT ti.id, ti.email::text
      INTO v_invitation_id, v_old_email
      FROM tenant_invitations ti
     WHERE ti.tenant_id = p_tenant_id
       AND ti.role = 'org_owner'
       AND ti.status = 'pending'
     FOR UPDATE;
    IF NOT FOUND THEN
        IF v_initialized THEN
            RAISE EXCEPTION 'pending_owner_invitation_not_found' USING ERRCODE = 'P0002';
        END IF;

        -- Recovery for a truly empty legacy shell: create its first privileged
        -- owner invitation rather than requiring an existing member target.
        INSERT INTO tenant_invitations (
            tenant_id, email, role, status, created_by, created_at
        ) VALUES (
            p_tenant_id, BTRIM(p_new_email), 'org_owner', 'pending',
            p_actor_user_id, v_created_at
        )
        RETURNING id INTO v_invitation_id;
        v_audit_action := 'platform.tenant.pending_owner_invitation_created';
    ELSE
        UPDATE tenant_invitations
           SET email = BTRIM(p_new_email), created_at = v_created_at
         WHERE id = v_invitation_id;
        v_audit_action := 'platform.tenant.pending_owner_invitation_replaced';
    END IF;

    UPDATE tenants
       SET ownership_initialized_at = COALESCE(ownership_initialized_at, v_created_at)
     WHERE id = p_tenant_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        p_tenant_id, p_actor_user_id,
        v_audit_action,
        'tenant_invitation', v_invitation_id,
        jsonb_build_object(
            'previous_email', v_old_email,
            'new_email', BTRIM(p_new_email)
        )
    );

    RETURN QUERY SELECT v_invitation_id, BTRIM(p_new_email), v_created_at;
END;
$$;

REVOKE ALL ON FUNCTION platform_replace_pending_owner_invitation(UUID, TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_replace_pending_owner_invitation(UUID, TEXT, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_replace_pending_owner_invitation(UUID, TEXT, UUID) TO aulalite_app;

-- Narrow RLS-safe directory used by the platform recovery UI. It reveals only
-- active owner/admin candidates in the selected tenant and independently
-- validates the platform actor.
CREATE OR REPLACE FUNCTION platform_list_tenant_owner_candidates(
    p_tenant_id UUID,
    p_actor_user_id UUID
)
RETURNS TABLE (
    user_id UUID,
    email TEXT,
    display_name TEXT,
    role TEXT
)
LANGUAGE plpgsql
SECURITY DEFINER
STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_authenticated_actor UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_initialized BOOLEAN;
BEGIN
    IF p_actor_user_id IS NULL
       OR v_authenticated_actor IS DISTINCT FROM p_actor_user_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;
    SELECT t.ownership_initialized_at IS NOT NULL
      INTO v_initialized
      FROM tenants t
     WHERE t.id = p_tenant_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tenant_not_found' USING ERRCODE = 'P0002';
    END IF;

    RETURN QUERY
    SELECT tm.user_id, u.email::text, u.display_name, tm.role
      FROM tenant_memberships tm
      JOIN users u ON u.id = tm.user_id
     WHERE tm.tenant_id = p_tenant_id
       AND tm.status = 'active'
       AND (
            tm.role IN ('org_owner', 'org_admin')
            OR (NOT v_initialized AND tm.role <> 'org_owner')
       )
       AND u.deleted_at IS NULL
     ORDER BY CASE tm.role
                  WHEN 'org_owner' THEN 0
                  WHEN 'org_admin' THEN 1
                  ELSE 2
              END,
              lower(u.email::text), tm.user_id;
END;
$$;

REVOKE ALL ON FUNCTION platform_list_tenant_owner_candidates(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_list_tenant_owner_candidates(UUID, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_list_tenant_owner_candidates(UUID, UUID) TO aulalite_app;

-- Audited break-glass recovery. This is intentionally separate from ordinary
-- organization transfer and validates the actor again inside the DB boundary.
CREATE OR REPLACE FUNCTION platform_recover_tenant_owner(
    p_tenant_id UUID,
    p_target_user_id UUID,
    p_actor_user_id UUID
)
RETURNS TABLE (
    previous_owner_user_id UUID,
    new_owner_user_id UUID,
    transferred_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_authenticated_actor UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_initialized BOOLEAN;
    v_previous UUID;
    v_transferred_at TIMESTAMPTZ := now();
BEGIN
    IF p_actor_user_id IS NULL
       OR v_authenticated_actor IS DISTINCT FROM p_actor_user_id
       OR NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id = p_actor_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'platform_admin_required' USING ERRCODE = '42501';
    END IF;

    SELECT t.ownership_initialized_at IS NOT NULL
      INTO v_initialized
      FROM tenants t
     WHERE t.id = p_tenant_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'tenant_not_found' USING ERRCODE = 'P0002';
    END IF;

    PERFORM 1
      FROM tenant_memberships tm
      JOIN users u ON u.id = tm.user_id AND u.deleted_at IS NULL
     WHERE tm.tenant_id = p_tenant_id
       AND tm.user_id = p_target_user_id
       AND tm.status = 'active'
       AND (
            (v_initialized AND tm.role = 'org_admin')
            OR (NOT v_initialized AND tm.role <> 'org_owner')
       )
     FOR UPDATE OF tm;
    IF NOT FOUND THEN
        IF v_initialized THEN
            RAISE EXCEPTION 'target_must_be_active_org_admin' USING ERRCODE = '23514';
        END IF;
        RAISE EXCEPTION 'target_must_be_active_member' USING ERRCODE = '23514';
    END IF;

    SELECT tm.user_id INTO v_previous
      FROM tenant_memberships tm
     WHERE tm.tenant_id = p_tenant_id
       AND tm.role = 'org_owner'
     FOR UPDATE;

    UPDATE tenant_memberships
       SET role = 'org_admin', updated_at = v_transferred_at
     WHERE tenant_id = p_tenant_id
       AND role = 'org_owner';
    UPDATE tenant_memberships
       SET role = 'org_owner', updated_at = v_transferred_at
     WHERE tenant_id = p_tenant_id AND user_id = p_target_user_id;
    UPDATE tenant_invitations
       SET status = 'revoked'
     WHERE tenant_id = p_tenant_id
       AND role = 'org_owner'
       AND status = 'pending';
    UPDATE tenants
       SET ownership_initialized_at = COALESCE(ownership_initialized_at, v_transferred_at)
     WHERE id = p_tenant_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        p_tenant_id, p_actor_user_id, 'platform.tenant.owner_recovered',
        'tenant', p_tenant_id,
        jsonb_build_object(
            'previous_owner_user_id', v_previous,
            'new_owner_user_id', p_target_user_id
        )
    );

    RETURN QUERY SELECT v_previous, p_target_user_id, v_transferred_at;
END;
$$;

REVOKE ALL ON FUNCTION platform_recover_tenant_owner(UUID, UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_recover_tenant_owner(UUID, UUID, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_recover_tenant_owner(UUID, UUID, UUID) TO aulalite_app;

-- Account erasure must transfer every organization ownership first.
CREATE OR REPLACE FUNCTION anonymize_user_account(
    p_user_id UUID,
    p_display_name TEXT,
    p_tombstone_email TEXT
)
RETURNS TABLE (anonymized_at TIMESTAMPTZ, deleted_at TIMESTAMPTZ)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_user_id UUID := NULLIF(current_setting('app.user_id', true), '')::uuid;
    v_user users%ROWTYPE;
    v_tenant UUID;
BEGIN
    IF p_user_id IS NULL OR v_actor_user_id IS DISTINCT FROM p_user_id THEN
        RAISE EXCEPTION 'authenticated_actor_mismatch' USING ERRCODE = '42501';
    END IF;
    PERFORM pg_advisory_xact_lock(7428561930174201);
    SELECT * INTO v_user FROM users WHERE id = p_user_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'user_not_found' USING ERRCODE = 'P0002';
    END IF;

    IF v_user.is_platform_admin
       AND v_user.identity_kind = 'global'
       AND v_user.identity_tenant_id IS NULL
       AND NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id <> p_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
           AND u.identity_kind = 'global'
           AND u.identity_tenant_id IS NULL
    ) THEN
        RAISE EXCEPTION 'last_platform_admin'
            USING ERRCODE = '23514', CONSTRAINT = 'users_last_platform_admin';
    END IF;

    FOR v_tenant IN
        SELECT tm.tenant_id FROM tenant_memberships tm
         WHERE tm.user_id = p_user_id AND tm.status = 'active'
         ORDER BY tm.tenant_id
    LOOP
        PERFORM id FROM tenants WHERE id = v_tenant FOR UPDATE;
    END LOOP;

    IF EXISTS (
        SELECT 1 FROM tenant_memberships mine
         WHERE mine.user_id = p_user_id
           AND mine.role = 'org_owner'
           AND mine.status = 'active'
    ) THEN
        RAISE EXCEPTION 'active_org_owner_cannot_be_erased'
            USING ERRCODE = '23514',
                  CONSTRAINT = 'tenant_memberships_active_org_owner_required';
    END IF;

    UPDATE tenant_memberships
       SET status = 'suspended', updated_at = now()
     WHERE user_id = p_user_id AND status = 'active';
    UPDATE users
       SET display_name = p_display_name,
           email = p_tombstone_email,
           avatar_url = NULL,
           locale = NULL,
           is_platform_admin = FALSE,
           deleted_at = COALESCE(deleted_at, now()),
           anonymized_at = COALESCE(anonymized_at, now()),
           tokens_valid_after = now()
     WHERE id = p_user_id;

    RETURN QUERY
    SELECT u.anonymized_at, u.deleted_at FROM users u WHERE u.id = p_user_id;
END;
$$;

REVOKE ALL ON FUNCTION anonymize_user_account(UUID, TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION anonymize_user_account(UUID, TEXT, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION anonymize_user_account(UUID, TEXT, TEXT) TO aulalite_app;
