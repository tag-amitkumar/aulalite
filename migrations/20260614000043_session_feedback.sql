-- Post-session feedback / ratings. One row per (session, user): a student rates
-- a live session 1..5 stars with an optional comment. Tenant-scoped under RLS
-- exactly like `announcements` (policy keys solely on app.tenant_id), so it is
-- enforced under the non-bypass `aulalite_app` role (20260517000020_app_role.sql).
--
-- UNIQUE(session_id, user_id) makes the write an upsert: a student editing their
-- rating updates the same row rather than inserting a duplicate.

CREATE TABLE session_feedback (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    rating INT NOT NULL CHECK (rating BETWEEN 1 AND 5),
    comment TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, user_id)
);

CREATE INDEX session_feedback_tenant_idx ON session_feedback (tenant_id);
-- Supports the staff summary aggregate (WHERE session_id = $1) and recent-comment
-- listing (ORDER BY created_at DESC).
CREATE INDEX session_feedback_session_created_idx ON session_feedback (session_id, created_at DESC);

ALTER TABLE session_feedback ENABLE ROW LEVEL SECURITY;
ALTER TABLE session_feedback FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON session_feedback
    USING (tenant_id::text = current_setting('app.tenant_id', true));

-- Match the explicit table grants the app role relies on (idempotent with the
-- default privileges from 20260517000020_app_role.sql; keeps the migration
-- self-contained).
GRANT SELECT, INSERT, UPDATE, DELETE ON session_feedback TO aulalite_app;
