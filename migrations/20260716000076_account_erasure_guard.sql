-- Account erasure is an authorization mutation as well as a privacy mutation.
-- Keep the identity tombstoned so a fresh provider token cannot restore it,
-- deactivate every tenant seat atomically, and preserve a recovery authority
-- for both the platform and each organization.

CREATE OR REPLACE FUNCTION anonymize_user_account(
    p_user_id UUID,
    p_display_name TEXT,
    p_tombstone_email TEXT
)
RETURNS TABLE (
    anonymized_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_user users%ROWTYPE;
    v_tenant UUID;
BEGIN
    -- Serialize the last-platform-owner check across every account deletion.
    -- Tenant rows provide the corresponding per-organization serialization.
    PERFORM pg_advisory_xact_lock(7428561930174201);

    SELECT * INTO v_user
      FROM users
     WHERE id = p_user_id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'user_not_found' USING ERRCODE = 'P0002';
    END IF;

    IF v_user.is_platform_admin AND NOT EXISTS (
        SELECT 1 FROM users u
         WHERE u.id <> p_user_id
           AND u.is_platform_admin
           AND u.deleted_at IS NULL
    ) THEN
        RAISE EXCEPTION 'last_platform_admin'
            USING ERRCODE = '23514',
                  CONSTRAINT = 'users_last_platform_admin';
    END IF;

    -- Lock every affected organization in a stable order. This is the same
    -- lock used by membership administration and the last-admin trigger.
    FOR v_tenant IN
        SELECT tm.tenant_id
          FROM tenant_memberships tm
         WHERE tm.user_id = p_user_id
           AND tm.status = 'active'
         ORDER BY tm.tenant_id
    LOOP
        PERFORM id FROM tenants WHERE id = v_tenant FOR UPDATE;
    END LOOP;

    IF EXISTS (
        SELECT 1
          FROM tenant_memberships mine
         WHERE mine.user_id = p_user_id
           AND mine.role = 'org_admin'
           AND mine.status = 'active'
           AND NOT EXISTS (
               SELECT 1
                 FROM tenant_memberships other
                WHERE other.tenant_id = mine.tenant_id
                  AND other.user_id <> p_user_id
                  AND other.role = 'org_admin'
                  AND other.status = 'active'
           )
    ) THEN
        RAISE EXCEPTION 'last_active_org_admin'
            USING ERRCODE = '23514',
                  CONSTRAINT = 'tenant_memberships_last_active_org_admin';
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
