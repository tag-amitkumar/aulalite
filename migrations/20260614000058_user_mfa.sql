-- Per-user TOTP MFA enrollment. PK = user_id (MFA is a property of the global
-- `users` account, which is tenant-agnostic). NOT tenant-scoped: access is
-- scoped to the OWNER via the app.user_id GUC (the same bootstrap key
-- tenant_memberships uses), PLUS a system_context SELECT so the auth middleware
-- can read `enabled` before tenant resolution (db::mfa::is_enabled sets
-- app.system='on' and FAILS OPEN). All writes flow through the owner path.
-- recovery_codes holds lowercase-hex SHA-256 hashes only (never plaintext).

CREATE TABLE user_mfa (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret BYTEA NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    verified_at TIMESTAMPTZ,
    recovery_codes TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE user_mfa ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_mfa FORCE ROW LEVEL SECURITY;
-- Owner-scoped: the row's user_id must match the bootstrap app.user_id GUC.
CREATE POLICY user_self_access ON user_mfa
    USING (user_id::text = current_setting('app.user_id', true))
    WITH CHECK (user_id::text = current_setting('app.user_id', true));
-- The auth middleware reads `enabled` cross-context (before tenant/user GUC is
-- fully established for this purpose) under app.system='on'.
CREATE POLICY system_context_select ON user_mfa
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON user_mfa TO aulalite_app;
