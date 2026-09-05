-- Remembered MFA devices. MFA is a global user property, not tenant-scoped.
-- The token itself is shown to the client once; the server stores only a hash.

CREATE TABLE user_mfa_trusted_devices (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    user_agent TEXT,
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX user_mfa_trusted_devices_user_active_idx
    ON user_mfa_trusted_devices (user_id, expires_at DESC)
    WHERE revoked_at IS NULL;

ALTER TABLE user_mfa_trusted_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_mfa_trusted_devices FORCE ROW LEVEL SECURITY;

CREATE POLICY trusted_device_owner_access ON user_mfa_trusted_devices
    USING (user_id::text = current_setting('app.user_id', true))
    WITH CHECK (user_id::text = current_setting('app.user_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON user_mfa_trusted_devices TO aulalite_app;
