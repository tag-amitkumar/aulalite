-- migrations/20260508000009_file_assets.sql
CREATE TABLE file_assets (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    owner_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    bucket TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','available','failed','pruned')),
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private','course','public')),
    linked_entity_type TEXT,
    linked_entity_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (bucket, object_key)
);

CREATE INDEX file_assets_owner_idx ON file_assets(owner_user_id);
CREATE INDEX file_assets_linked_idx ON file_assets(linked_entity_type, linked_entity_id);

ALTER TABLE file_assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE file_assets FORCE ROW LEVEL SECURITY;

CREATE POLICY file_assets_tenant_isolation ON file_assets
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
