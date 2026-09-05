-- Preserve an organization recovery path even when membership rows are
-- changed outside the HTTP admin endpoint (SSO/invitation upserts, maintenance
-- scripts, or future code). The tenant row is the shared per-organization lock,
-- so concurrent attempts to demote two different admins cannot both succeed.

CREATE OR REPLACE FUNCTION protect_last_active_org_admin()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF OLD.role = 'org_admin' AND OLD.status = 'active' THEN
        IF TG_OP = 'UPDATE' THEN
            IF NEW.role = 'org_admin' AND NEW.status = 'active' THEN
                RETURN NEW;
            END IF;
        END IF;

        -- Match the application mutation lock. The row exists for every valid
        -- membership because of the FK, and locking it serializes the count.
        PERFORM id FROM tenants WHERE id = OLD.tenant_id FOR UPDATE;

        -- A tenant deletion intentionally cascades through every membership.
        -- Once the parent row is gone there is no organization to lock out, so
        -- let that cascade proceed while still guarding direct row deletes.
        IF NOT FOUND THEN
            RETURN OLD;
        END IF;

        IF NOT EXISTS (
            SELECT 1
              FROM tenant_memberships
             WHERE tenant_id = OLD.tenant_id
               AND user_id <> OLD.user_id
               AND role = 'org_admin'
               AND status = 'active'
        ) THEN
            RAISE EXCEPTION 'last_active_org_admin'
                USING ERRCODE = '23514',
                      CONSTRAINT = 'tenant_memberships_last_active_org_admin';
        END IF;
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS tenant_memberships_protect_last_admin ON tenant_memberships;
CREATE TRIGGER tenant_memberships_protect_last_admin
BEFORE UPDATE OF role, status OR DELETE ON tenant_memberships
FOR EACH ROW
EXECUTE FUNCTION protect_last_active_org_admin();

REVOKE ALL ON FUNCTION protect_last_active_org_admin() FROM PUBLIC;
