-- In-class live polls (history). One row per poll, scoped to (tenant_id,
-- session_id). The live poll is broker-authoritative (ephemeral, Redis/Mock);
-- this table is OPTIONAL after-class history, written best-effort from the
-- live-session WebSocket handler and never blocking the real-time poll.
-- Tenant-scoped under RLS exactly like announcements / live_room_messages
-- (policy keys solely on app.tenant_id), enforced under the non-bypass
-- aulalite_app role (20260517000020_app_role.sql).
--   options : JSON array of option label strings (2..=6 entries)
--   counts  : JSON array of ints, same length as options; counts[i] is the
--             vote tally for options[i]. Updated as votes arrive and at end.

CREATE TABLE live_session_polls (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    created_by UUID NOT NULL REFERENCES users(id),
    question TEXT NOT NULL,
    options JSONB NOT NULL,
    counts JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ
);

CREATE INDEX live_session_polls_tenant_idx ON live_session_polls (tenant_id);
CREATE INDEX live_session_polls_session_created_idx
    ON live_session_polls (session_id, created_at);

ALTER TABLE live_session_polls ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_session_polls FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON live_session_polls
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON live_session_polls TO aulalite_app;
