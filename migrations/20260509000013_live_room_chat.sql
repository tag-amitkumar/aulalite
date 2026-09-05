-- migrations/20260509000013_live_room_chat.sql
-- Phase 1b-γ: live room UX — chat persistence, kick history, per-student
-- publish nonces for hand-raise audio promote.

CREATE TABLE live_room_messages (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    sender_user_id UUID NOT NULL REFERENCES users(id),
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ,
    deleted_by_user_id UUID REFERENCES users(id)
);
CREATE INDEX live_room_messages_session_idx
    ON live_room_messages (session_id, created_at);
CREATE INDEX live_room_messages_prune_idx
    ON live_room_messages (created_at)
    WHERE deleted_at IS NULL;

ALTER TABLE live_room_messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_room_messages FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON live_room_messages
    USING (tenant_id::text = current_setting('app.tenant_id', true));

CREATE TABLE live_room_kicks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    kicked_by_user_id UUID NOT NULL REFERENCES users(id),
    kicked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, user_id)
);

ALTER TABLE live_room_kicks ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_room_kicks FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON live_room_kicks
    USING (tenant_id::text = current_setting('app.tenant_id', true));

ALTER TABLE live_sessions
    ADD COLUMN student_publish_nonces JSONB NOT NULL DEFAULT '{}';
