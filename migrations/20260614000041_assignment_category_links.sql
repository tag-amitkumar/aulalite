-- Weighted gradebook: assignment→category links. A LINK table (rather than a
-- column on `assignments`) deliberately avoids altering the assignments table,
-- so this lands without colliding with concurrent assignment work.
--
-- An assignment belongs to at most one category, so `assignment_id` is the
-- PRIMARY KEY (one link per assignment; re-assigning UPSERTs). Both columns are
-- tenant-scoped under RLS exactly like `announcements` (policy keys solely on
-- app.tenant_id), enforced under the non-bypass `aulalite_app` role. The
-- category FK cascades on delete so dropping a category cleanly unlinks its
-- assignments; the assignment FK cascades so deleting an assignment removes its
-- link.

CREATE TABLE assignment_category_links (
    assignment_id UUID PRIMARY KEY REFERENCES assignments(id) ON DELETE CASCADE,
    category_id UUID NOT NULL REFERENCES assignment_categories(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX assignment_category_links_tenant_idx ON assignment_category_links (tenant_id);
-- Supports the per-course matrix join and the cascade-aware per-category lookup.
CREATE INDEX assignment_category_links_course_idx ON assignment_category_links (course_id);
CREATE INDEX assignment_category_links_category_idx ON assignment_category_links (category_id);

ALTER TABLE assignment_category_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE assignment_category_links FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON assignment_category_links
    USING (tenant_id::text = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE, DELETE ON assignment_category_links TO aulalite_app;
