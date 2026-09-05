-- Enforce tenant seat caps while accepting parent invitations during the
-- cross-tenant JIT provisioning flow. Parent invitations are not reservations,
-- so a seat is checked at acceptance time. Relevant tenant rows are locked in
-- UUID order to serialize with every other seat-consuming path and avoid
-- cross-tenant deadlocks.

ALTER TABLE parent_invitations
    ADD COLUMN seat_reserved BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX parent_invitations_reserved_email_idx
    ON parent_invitations (tenant_id, lower(parent_email))
    WHERE status = 'pending' AND seat_reserved;

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
    r_tenant RECORD;
    v_status TEXT;
    v_active BIGINT;
    v_pending BIGINT;
    v_cap BIGINT;
    v_overage TEXT;
    v_has_reserved_seat BOOLEAN;
    v_rows INTEGER;
BEGIN
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
        SELECT tm.status
          INTO v_status
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
            -- user's seat. The later member-invitation acceptance in the same
            -- JIT transaction will convert that reservation to active.
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

-- Close the legacy privileged entry point. Keep its INTEGER contract for a
-- rolling release with an older backend, but delegate every write to v2.
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
