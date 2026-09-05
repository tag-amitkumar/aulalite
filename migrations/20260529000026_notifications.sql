-- migrations/20260529000026_notifications.sql
-- Notifications data foundation: in-app notifications, push device tokens, and
-- per-user notification preferences.
--
-- Three tables with deliberately different scoping (mirrors the billing
-- migration's "different scoping per concern" approach):
--   * notifications          — TENANT-SCOPED, per-user. Strict tenant_isolation
--                              RLS + FORCE so the non-bypass `aulalite_app` role
--                              only ever sees its own tenant's rows. Handlers
--                              additionally scope by user_id so a caller only
--                              reads/mutates their OWN notifications.
--   * device_tokens          — TENANT-SCOPED, per-user push targets. Same RLS as
--                              notifications. `token` is globally UNIQUE so a
--                              device that re-registers upserts in place.
--   * notification_preferences — GLOBAL per-user (PK = user_id), like `users`:
--                              NO tenant_id, NO RLS. A user's channel prefs are
--                              cross-tenant; readability/writability is enforced
--                              in the handler (scoped to ctx.user_id), exactly as
--                              the spec requires.

-- ---------------------------------------------------------------------------
-- notifications — tenant-scoped, per-user in-app feed
-- ---------------------------------------------------------------------------
CREATE TABLE notifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    link TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at TIMESTAMPTZ
);

-- The hot read path is "this user's unread/newest notifications".
CREATE INDEX notifications_user_read_idx ON notifications (user_id, read_at);
CREATE INDEX notifications_tenant_idx ON notifications (tenant_id);

ALTER TABLE notifications ENABLE ROW LEVEL SECURITY;
ALTER TABLE notifications FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON notifications
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON notifications TO aulalite_app;

-- ---------------------------------------------------------------------------
-- device_tokens — tenant-scoped push registration targets, one row per token
-- ---------------------------------------------------------------------------
CREATE TABLE device_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id UUID NOT NULL REFERENCES users(id),
    token TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL CHECK (platform IN ('web','ios','android')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX device_tokens_user_idx ON device_tokens (user_id);
CREATE INDEX device_tokens_tenant_idx ON device_tokens (tenant_id);

ALTER TABLE device_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_tokens FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON device_tokens
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON device_tokens TO aulalite_app;

-- ---------------------------------------------------------------------------
-- notification_preferences — GLOBAL per-user (no tenant_id, no RLS, like users)
--
-- A row is OPTIONAL: absence means "all channels enabled" (the Rust layer
-- defaults to all-true when no row exists). The owning user reads/writes their
-- own row via handler scoping (ctx.user_id); there is no tenant to gate on.
-- ---------------------------------------------------------------------------
CREATE TABLE notification_preferences (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    email_enabled BOOLEAN NOT NULL DEFAULT true,
    push_enabled BOOLEAN NOT NULL DEFAULT true,
    in_app_enabled BOOLEAN NOT NULL DEFAULT true,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Global table: grant read/write to the app role explicitly (it has no
-- tenant_id to gate on; handler scoping by user_id is the access control).
GRANT SELECT, INSERT, UPDATE, DELETE ON notification_preferences TO aulalite_app;
