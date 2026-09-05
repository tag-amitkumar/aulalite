-- Membership-scoped workspace directory used by the authenticated workspace
-- switcher. Tenant names are RLS-protected, while the caller must be able to
-- see every workspace they actively belong to before selecting one. Keep the
-- cross-tenant read inside this narrow, user-scoped SECURITY DEFINER function.
CREATE OR REPLACE FUNCTION list_my_workspaces()
    RETURNS TABLE (
        tenant_id uuid,
        name text,
        slug text,
        role text
    )
    LANGUAGE sql
    STABLE
    SECURITY DEFINER
    SET search_path = public
AS $$
    SELECT tm.tenant_id, t.name, t.slug, tm.role
      FROM tenant_memberships tm
      JOIN tenants t ON t.id = tm.tenant_id
     WHERE tm.user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
       AND tm.status = 'active'
     ORDER BY lower(t.name), tm.joined_at, tm.tenant_id
$$;

REVOKE ALL ON FUNCTION list_my_workspaces() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite;
GRANT EXECUTE ON FUNCTION list_my_workspaces() TO aulalite_app;
