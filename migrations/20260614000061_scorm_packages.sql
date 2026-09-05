-- Registered SCORM packages, scoped to (tenant_id, course_id). The uploaded
-- .zip lives in file_assets (purpose 'scorm'); we record the parsed launch href
-- + version here. Tenant-scoped under RLS exactly like `announcements`.

CREATE TABLE scorm_packages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    asset_id UUID NOT NULL REFERENCES file_assets(id),
    scorm_version TEXT NOT NULL CHECK (scorm_version IN ('1.2','2004')),
    launch_href TEXT NOT NULL,
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX scorm_packages_tenant_idx ON scorm_packages (tenant_id);
CREATE INDEX scorm_packages_course_idx ON scorm_packages (course_id, created_at DESC);

ALTER TABLE scorm_packages ENABLE ROW LEVEL SECURITY;
ALTER TABLE scorm_packages FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON scorm_packages
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON scorm_packages TO aulalite_app;
