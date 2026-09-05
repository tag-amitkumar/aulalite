-- Give every workspace a concrete entitlement. Existing active organizations
-- are grandfathered onto Starter; existing trials receive a fixed 14-day end.
UPDATE tenants
   SET trial_ends_at = now() + interval '14 days'
 WHERE status = 'trialing' AND trial_ends_at IS NULL;

INSERT INTO subscriptions (
    tenant_id, plan_id, status, trial_ends_at, overage_behavior
)
SELECT t.id,
       'starter',
       CASE
           WHEN t.status = 'active' THEN 'active'
           WHEN t.status = 'suspended' THEN 'canceled'
           ELSE 'trialing'
       END,
       CASE WHEN t.status = 'trialing' THEN t.trial_ends_at ELSE NULL END,
       'block'
  FROM tenants t
 WHERE NOT EXISTS (SELECT 1 FROM subscriptions s WHERE s.tenant_id = t.id);

UPDATE subscriptions SET overage_behavior = 'block' WHERE overage_behavior <> 'block';
ALTER TABLE subscriptions DROP CONSTRAINT IF EXISTS subscriptions_overage_behavior_check;
ALTER TABLE subscriptions
    ADD CONSTRAINT subscriptions_overage_behavior_check
    CHECK (overage_behavior = 'block');

-- One policy function is shared by browser auth, API keys, SSO/LTI lookup and
-- the workspace directory. Past-due customers receive a short recovery grace;
-- canceled, suspended, and expired-trial workspaces fail closed.
CREATE OR REPLACE FUNCTION tenant_access_allowed(p_tenant_id UUID)
RETURNS BOOLEAN
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM tenants t
          JOIN subscriptions s ON s.tenant_id = t.id
         WHERE t.id = p_tenant_id
           AND t.status IN ('active', 'trialing')
           AND (
               s.status = 'active'
               OR (s.status = 'past_due' AND s.updated_at > now() - interval '7 days')
               OR (
                   s.status = 'trialing'
                   AND COALESCE(s.trial_ends_at, t.trial_ends_at) > now()
               )
           )
    )
$$;

REVOKE ALL ON FUNCTION tenant_access_allowed(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION tenant_access_allowed(UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION tenant_access_allowed(UUID) TO aulalite_app;

CREATE OR REPLACE FUNCTION resolve_active_workspace(
    p_user_id UUID,
    p_requested_tenant_id UUID DEFAULT NULL
)
RETURNS TABLE (tenant_id UUID, role TEXT)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT tm.tenant_id, tm.role
      FROM tenant_memberships tm
     WHERE tm.user_id = p_user_id
       AND tm.status = 'active'
       AND (p_requested_tenant_id IS NULL OR tm.tenant_id = p_requested_tenant_id)
       AND tenant_access_allowed(tm.tenant_id)
     ORDER BY tm.joined_at, tm.tenant_id
     LIMIT 1
$$;

REVOKE ALL ON FUNCTION resolve_active_workspace(UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_active_workspace(UUID, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION resolve_active_workspace(UUID, UUID) TO aulalite_app;

CREATE OR REPLACE FUNCTION list_my_workspaces()
RETURNS TABLE (tenant_id UUID, name TEXT, slug TEXT, role TEXT)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT tm.tenant_id, t.name, t.slug, tm.role
      FROM tenant_memberships tm
      JOIN tenants t ON t.id = tm.tenant_id
     WHERE tm.user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
       AND tm.status = 'active'
       AND tenant_access_allowed(tm.tenant_id)
     ORDER BY lower(t.name), tm.joined_at, tm.tenant_id
$$;

REVOKE ALL ON FUNCTION list_my_workspaces() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite_app;

-- Provision tenant, fixed trial entitlement, first owner invitation and audit
-- row in one transaction. A retry after an email-provider failure is
-- idempotent only when the same pending owner invitation exists.
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
    v_tenant_id UUID;
    v_invitation_id UUID;
BEGIN
    SELECT id INTO v_tenant_id FROM tenants WHERE slug = p_slug;
    IF v_tenant_id IS NOT NULL THEN
        IF EXISTS (
            SELECT 1 FROM tenant_invitations ti
             WHERE ti.tenant_id = v_tenant_id
               AND ti.status = 'pending'
               AND ti.role = 'org_admin'
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
    VALUES (v_tenant_id, p_admin_email, 'org_admin', p_actor_user_id)
    RETURNING id INTO v_invitation_id;

    INSERT INTO audit_events (
        tenant_id, actor_user_id, action, resource_type, resource_id, metadata
    ) VALUES (
        v_tenant_id, p_actor_user_id, 'platform.tenant.create', 'tenant',
        v_tenant_id,
        jsonb_build_object(
            'slug', p_slug,
            'admin_email', p_admin_email,
            'invitation_id', v_invitation_id
        )
    );

    RETURN v_tenant_id;
END;
$$;

REVOKE ALL ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) TO aulalite;
GRANT EXECUTE ON FUNCTION platform_provision_tenant(TEXT, TEXT, TEXT, UUID) TO aulalite_app;
