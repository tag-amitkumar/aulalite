-- Lesson prerequisites + drip/time-gating (Wave 4: course structure & content).
-- A lesson may require other lessons to be completed first (lesson_prerequisites)
-- and/or carry a future release_at (drip). For a STUDENT a lesson is LOCKED when
-- release_at is still in the future OR any required lesson is not yet completed
-- (handler-computed against lesson_completions). Staff are never gated.
--
-- lesson_prerequisites is tenant-scoped under RLS exactly like `announcements`
-- (policy keys solely on app.tenant_id), enforced under the non-bypass
-- `aulalite_app` role (20260517000020_app_role.sql).

-- (1) Drip / time-gate column on lessons. Nullable; NULL = no time-gate.
ALTER TABLE lessons ADD COLUMN IF NOT EXISTS release_at TIMESTAMPTZ;

-- (2) Prerequisite edges: `lesson_id` requires `required_lesson_id`.
CREATE TABLE lesson_prerequisites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    lesson_id UUID NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
    required_lesson_id UUID NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT lesson_prerequisites_no_self CHECK (lesson_id <> required_lesson_id),
    UNIQUE (lesson_id, required_lesson_id)
);

CREATE INDEX lesson_prerequisites_tenant_idx ON lesson_prerequisites (tenant_id);
-- Supports both the per-lesson list (WHERE lesson_id = $1) and the lock-state
-- join (LEFT JOIN ... ON lp.lesson_id = l.id).
CREATE INDEX lesson_prerequisites_lesson_idx ON lesson_prerequisites (lesson_id);
CREATE INDEX lesson_prerequisites_required_idx ON lesson_prerequisites (required_lesson_id);

ALTER TABLE lesson_prerequisites ENABLE ROW LEVEL SECURITY;
ALTER TABLE lesson_prerequisites FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON lesson_prerequisites
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON lesson_prerequisites TO aulalite_app;
