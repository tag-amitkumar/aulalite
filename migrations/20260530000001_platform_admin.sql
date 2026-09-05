-- migrations/20260530000001_platform_admin.sql
-- Platform super-admin data layer: cross-tenant reads + production tenant
-- provisioning for the Elementors staff role (gated in Rust by
-- RequestContext.is_platform_admin, NOT a TenantRole).
--
-- These operations span ALL tenants, so they cannot be satisfied by a single
-- per-request `app.tenant_id` GUC under the non-bypass `aulalite_app` role:
--   * tenants carries the `tenants_self_access` RLS policy (id = app.tenant_id);
--   * tenant_memberships / subscriptions carry strict tenant_isolation RLS.
-- A platform admin may also have NO tenant membership at all, so there is no
-- meaningful single tenant to scope to.
--
-- We therefore mirror the codebase's sanctioned cross-tenant escape — a
-- SECURITY DEFINER function owned by the table-owning migration role — exactly
-- like accept_parent_invitations_for_email / accept_tenant_invitations_for_email
-- (20260529000023_parent_links.sql, 20260529000025_tenant_invitations.sql). The
-- functions run with the definer's privileges and bypass the per-tenant RLS for
-- that one server-side step. Each one REVOKEs PUBLIC and GRANTs EXECUTE to the
-- same app roles those existing helpers use (`aulalite`, `aulalite_app`).
--
-- The org_admin invite that ties a freshly provisioned tenant to its first
-- admin is created in Rust via db::member_invitations::create_invitation (a
-- normal tenant-scoped write under the new tenant's GUC), NOT here.

-- ---------------------------------------------------------------------------
-- platform_list_tenants() — cross-tenant tenant directory.
--
-- One row per tenant with: active member count (LEFT JOIN tenant_memberships
-- WHERE status='active') and the subscription plan_id (LEFT JOIN subscriptions),
-- newest first. SECURITY DEFINER so it reads across every tenant's RLS-protected
-- rows.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION platform_list_tenants()
    RETURNS TABLE (
        id uuid,
        slug text,
        name text,
        status text,
        created_at timestamptz,
        member_count bigint,
        plan_id text
    )
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
BEGIN
    RETURN QUERY
        SELECT t.id,
               t.slug,
               t.name,
               t.status,
               t.created_at,
               COALESCE(m.member_count, 0) AS member_count,
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

REVOKE ALL ON FUNCTION platform_list_tenants() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_list_tenants() TO aulalite;
GRANT EXECUTE ON FUNCTION platform_list_tenants() TO aulalite_app;

-- ---------------------------------------------------------------------------
-- platform_get_tenant_summary(p_id) — single-tenant re-read for a mutation
-- response. Same shape as platform_list_tenants, filtered to one tenant. NULL
-- row (no rows returned) if the id does not exist.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION platform_get_tenant_summary(p_id uuid)
    RETURNS TABLE (
        id uuid,
        slug text,
        name text,
        status text,
        created_at timestamptz,
        member_count bigint,
        plan_id text
    )
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
BEGIN
    RETURN QUERY
        SELECT t.id,
               t.slug,
               t.name,
               t.status,
               t.created_at,
               COALESCE(m.member_count, 0) AS member_count,
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

REVOKE ALL ON FUNCTION platform_get_tenant_summary(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_get_tenant_summary(uuid) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_get_tenant_summary(uuid) TO aulalite_app;

-- ---------------------------------------------------------------------------
-- platform_create_tenant(p_slug, p_name, p_status) — provision a new tenant.
-- Inserts a tenants row and returns its id. `p_status` defaults to 'trialing'
-- when NULL/empty and is validated against the tenants CHECK constraint here
-- (the Rust layer also validates). Raises a clear error on a duplicate slug so
-- the handler can map it to a 4xx rather than a generic 500.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION platform_create_tenant(
        p_slug text,
        p_name text,
        p_status text
    )
    RETURNS uuid
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
DECLARE
    v_status text := COALESCE(NULLIF(p_status, ''), 'trialing');
    v_id uuid;
BEGIN
    IF v_status NOT IN ('active', 'trialing', 'suspended') THEN
        RAISE EXCEPTION 'invalid tenant status: %', v_status
            USING ERRCODE = 'check_violation';
    END IF;

    IF EXISTS (SELECT 1 FROM tenants WHERE slug = p_slug) THEN
        RAISE EXCEPTION 'tenant slug already exists: %', p_slug
            USING ERRCODE = 'unique_violation';
    END IF;

    INSERT INTO tenants (slug, name, status)
    VALUES (p_slug, p_name, v_status)
    RETURNING id INTO v_id;

    RETURN v_id;
END;
$$;

REVOKE ALL ON FUNCTION platform_create_tenant(text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_create_tenant(text, text, text) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_create_tenant(text, text, text) TO aulalite_app;

-- ---------------------------------------------------------------------------
-- platform_set_tenant_status(p_id, p_status) — flip a tenant's lifecycle
-- status. Validates the enum, returns true if a row was updated, false if the
-- id does not exist.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION platform_set_tenant_status(
        p_id uuid,
        p_status text
    )
    RETURNS boolean
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
DECLARE
    v_updated integer;
BEGIN
    IF p_status NOT IN ('active', 'trialing', 'suspended') THEN
        RAISE EXCEPTION 'invalid tenant status: %', p_status
            USING ERRCODE = 'check_violation';
    END IF;

    UPDATE tenants
       SET status = p_status,
           updated_at = now()
     WHERE id = p_id;

    GET DIAGNOSTICS v_updated = ROW_COUNT;
    RETURN v_updated > 0;
END;
$$;

REVOKE ALL ON FUNCTION platform_set_tenant_status(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_set_tenant_status(uuid, text) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_set_tenant_status(uuid, text) TO aulalite_app;
