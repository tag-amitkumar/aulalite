-- migrations/20260529000022_attendance.sql
-- Phase 2: durable live-room attendance. Live presence is otherwise ephemeral
-- (Redis only); this table persists per-(session,user) join/leave accounting so
-- analytics + the parent dashboard have a source of truth.
--
-- One row per (session_id, user_id). `last_joined_at` is internal accounting
-- (the most-recent join) used to accumulate `total_seconds` on leave/finalize.
-- `open = true` means the user is currently joined; the session-end reconciler
-- (`finalize_open_for_session`) closes any rows left open by a dropped socket.

CREATE TABLE attendance (
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    first_joined_at TIMESTAMPTZ NOT NULL,
    last_joined_at TIMESTAMPTZ NOT NULL,
    last_left_at TIMESTAMPTZ,
    total_seconds INT NOT NULL DEFAULT 0,
    reconnect_count INT NOT NULL DEFAULT 0,
    open BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, user_id)
);

CREATE INDEX attendance_tenant_idx ON attendance (tenant_id);
CREATE INDEX attendance_session_idx ON attendance (session_id);

ALTER TABLE attendance ENABLE ROW LEVEL SECURITY;
ALTER TABLE attendance FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON attendance
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Match the explicit table grants the app role relies on. The default
-- privileges from 20260517000020_app_role.sql already cover newly-created
-- tables, but granting here keeps the migration self-contained and idempotent.
GRANT SELECT, INSERT, UPDATE, DELETE ON attendance TO aulalite_app;
