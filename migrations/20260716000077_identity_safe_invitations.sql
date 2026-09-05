-- Parent/child relationships are tenant-local. The former two-column key
-- silently discarded a valid relationship when the same pair belonged to two
-- schools.
ALTER TABLE parent_links DROP CONSTRAINT IF EXISTS parent_links_pkey;
ALTER TABLE parent_links
    ADD CONSTRAINT parent_links_pkey
    PRIMARY KEY (tenant_id, parent_user_id, student_user_id);

-- Accept member invitations without overriding an administrator suspension.
-- Every affected tenant is locked in UUID order, matching all other
-- seat-consuming mutations and avoiding cross-tenant deadlocks.
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
    v_count INTEGER := 0;
    v_status TEXT;
    r RECORD;
BEGIN
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
        -- SELECT INTO leaves the prior value untouched when no membership is
        -- found, so reset it before inspecting each invitation.
        v_status := NULL;
        SELECT tm.status
          INTO v_status
          FROM tenant_memberships tm
         WHERE tm.tenant_id = r.tenant_id
           AND tm.user_id = p_user_id
         FOR UPDATE;

        IF v_status = 'suspended' THEN
            UPDATE tenant_invitations
               SET status = 'revoked'
             WHERE id = r.id;
            CONTINUE;
        END IF;

        INSERT INTO tenant_memberships
            (tenant_id, user_id, role, status)
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
