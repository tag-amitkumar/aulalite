-- Reminder de-dup ledger for the calendar reminder sweep + the system-context
-- RLS the cross-tenant sweep needs to read courses/assignments/course_memberships.
--
-- reminders_sent: one row per (entity_type, entity_id, reminder_kind) that has
-- been notified, so the sweep skips it next pass. Tenant-scoped under RLS like
-- `announcements` (policy keys solely on app.tenant_id) for normal access, PLUS
-- permissive system_context policies so the trusted in-process sweep (which sets
-- app.system='on' via db::begin_system_context, no tenant GUC) can SELECT for the
-- NOT EXISTS dedup check and INSERT the ledger row. Mirrors 20260530000030.

CREATE TABLE reminders_sent (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    -- 'session' | 'assignment'
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    -- 'session_starting' | 'assignment_due'
    reminder_kind TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Idempotency: a given reminder for a given entity is recorded at most once.
CREATE UNIQUE INDEX reminders_sent_unique_idx
    ON reminders_sent (entity_type, entity_id, reminder_kind);
CREATE INDEX reminders_sent_tenant_idx ON reminders_sent (tenant_id);

ALTER TABLE reminders_sent ENABLE ROW LEVEL SECURITY;
ALTER TABLE reminders_sent FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON reminders_sent
    USING (tenant_id::text = current_setting('app.tenant_id', true));
-- The reminder sweep runs cross-tenant under app.system='on' (no tenant GUC):
-- permit SELECT (NOT EXISTS dedup) + INSERT (ledger write) in that context only.
CREATE POLICY system_context_select ON reminders_sent
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_insert ON reminders_sent
    FOR INSERT
    WITH CHECK (current_setting('app.system', true) = 'on');

GRANT SELECT, INSERT, UPDATE, DELETE ON reminders_sent TO aulalite_app;

-- The reminder sweep's discovery queries JOIN courses + filter assignments and
-- read course_memberships with NO app.tenant_id set, so their tenant_isolation
-- policies would filter every row to NULL under the non-bypass aulalite_app role.
-- Add permissive SELECT-only system_context policies (OR'd with tenant_isolation;
-- they ONLY add access when app.system='on', which exclusively the sweep sets).
CREATE POLICY system_context_select ON courses
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_select ON assignments
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
CREATE POLICY system_context_select ON course_memberships
    FOR SELECT
    USING (current_setting('app.system', true) = 'on');
