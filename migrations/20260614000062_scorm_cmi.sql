-- Per-(package, user) SCORM CMI runtime state, stored as a JSON blob (flat
-- element-path -> value map written by the JS bridge on Commit/Finish).
-- Tenant-scoped under RLS exactly like `announcements`. One row per learner per
-- package (unique constraint backs the upsert in db::scorm::save_cmi).

CREATE TABLE scorm_cmi (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    package_id UUID NOT NULL REFERENCES scorm_packages(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    cmi JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (package_id, user_id)
);

CREATE INDEX scorm_cmi_tenant_idx ON scorm_cmi (tenant_id);
CREATE INDEX scorm_cmi_user_idx ON scorm_cmi (user_id);

ALTER TABLE scorm_cmi ENABLE ROW LEVEL SECURITY;
ALTER TABLE scorm_cmi FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON scorm_cmi
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON scorm_cmi TO aulalite_app;
