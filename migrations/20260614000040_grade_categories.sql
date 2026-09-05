-- Weighted gradebook: grade categories. One row per category, scoped to
-- (tenant_id, course_id). Categories group assignments so the gradebook can
-- compute a weighted total per student. Staff-managed (handler-gated on
-- caller_can_staff_course). Tenant-scoped under RLS exactly like `announcements`
-- (policy keys solely on app.tenant_id), so it is enforced under the non-bypass
-- `aulalite_app` role (20260517000020_app_role.sql).
--
-- `weight_percent` is a plain integer (0..=100) — the intended share of the
-- final grade for this category. Weights are NOT forced to sum to 100; the
-- gradebook normalizes by the sum of the weights of categories that actually
-- have a graded assignment for the student, so partial coursework still yields
-- a sensible running total.

CREATE TABLE assignment_categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    weight_percent INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX assignment_categories_tenant_idx ON assignment_categories (tenant_id);
-- Supports the per-course list query (WHERE course_id = $1 ORDER BY name).
CREATE INDEX assignment_categories_course_idx ON assignment_categories (course_id);

ALTER TABLE assignment_categories ENABLE ROW LEVEL SECURITY;
ALTER TABLE assignment_categories FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON assignment_categories
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON assignment_categories TO aulalite_app;
