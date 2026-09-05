-- Transactional, RLS-safe provisioning for self-service academy owners.
--
-- `personal_owner_user_id` is the durable idempotency key. A membership can be
-- suspended or accidentally removed, but a retry must never create a second
-- personal workspace for the same identity.
ALTER TABLE tenants
    ADD COLUMN personal_owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX tenants_personal_owner_unique
    ON tenants (personal_owner_user_id)
    WHERE personal_owner_user_id IS NOT NULL;

-- Called only from the authenticated JIT provisioning transaction, after all
-- pending tenant invitations have been accepted. SECURITY DEFINER is required
-- because this bootstrap step intentionally has no tenant GUC yet and must see
-- memberships across tenants before deciding whether to create a workspace.
-- The function owns the full check/create/membership operation and serializes
-- per user, preserving RLS and making concurrent first requests idempotent.
CREATE OR REPLACE FUNCTION provision_personal_workspace_if_needed(
        p_user_id UUID,
        p_workspace_name TEXT
    )
    RETURNS UUID
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = public
AS $$
DECLARE
    v_tenant_id UUID;
    v_workspace_name TEXT;
BEGIN
    IF p_user_id IS NULL OR NOT EXISTS (SELECT 1 FROM users WHERE id = p_user_id) THEN
        RAISE EXCEPTION 'valid user is required'
            USING ERRCODE = 'foreign_key_violation';
    END IF;

    -- The namespace prefix prevents this lock from colliding with unrelated
    -- advisory-lock users. It is released automatically with the transaction.
    PERFORM pg_advisory_xact_lock(
        hashtextextended('aulalite:personal-workspace:' || p_user_id::text, 0)
    );

    -- An accepted invitation (or any existing membership state) means this is
    -- an invited account. Do not silently give it a separate owner workspace.
    SELECT tm.tenant_id
      INTO v_tenant_id
      FROM tenant_memberships tm
     WHERE tm.user_id = p_user_id
     ORDER BY tm.joined_at ASC, tm.tenant_id ASC
     LIMIT 1;

    IF v_tenant_id IS NOT NULL THEN
        RETURN NULL;
    END IF;

    -- Recover an owner membership if it was removed between retries. The
    -- unique owner index remains the ultimate exactly-once invariant.
    SELECT t.id
      INTO v_tenant_id
      FROM tenants t
     WHERE t.personal_owner_user_id = p_user_id;

    v_workspace_name := LEFT(
        COALESCE(NULLIF(BTRIM(p_workspace_name), ''), 'My academy'),
        160
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

        UPDATE tenants
           SET trial_ends_at = now() + interval '14 days'
         WHERE id = v_tenant_id;

        INSERT INTO subscriptions (
            tenant_id, plan_id, status, trial_ends_at, overage_behavior
        )
        VALUES (
            v_tenant_id, 'starter', 'trialing',
            now() + interval '14 days', 'block'
        );
    END IF;

    INSERT INTO tenant_memberships (tenant_id, user_id, role, status)
    VALUES (v_tenant_id, p_user_id, 'org_admin', 'active')
    ON CONFLICT (tenant_id, user_id) DO UPDATE
        SET role = 'org_admin',
            status = 'active',
            updated_at = now();

    RETURN v_tenant_id;
END;
$$;

REVOKE ALL ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION provision_personal_workspace_if_needed(UUID, TEXT) TO aulalite_app;
