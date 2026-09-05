-- migrations/20260508000011_tighten_file_assets_links.sql

-- Wire deferred FKs from Phase 1a Migration 0009 (file_assets) back to courses + lessons.
-- ON DELETE SET NULL because asset deletion shouldn't cascade-remove the parent course/lesson.
ALTER TABLE courses
    ADD CONSTRAINT courses_cover_asset_id_fkey
    FOREIGN KEY (cover_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

ALTER TABLE lessons
    ADD CONSTRAINT lessons_video_asset_id_fkey
    FOREIGN KEY (video_asset_id) REFERENCES file_assets(id) ON DELETE SET NULL;

-- Tighten polymorphic linked_entity_type to known values. Forward-compatible:
-- new consumers extend the CHECK in their own migration.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN ('course', 'lesson'));

-- Defensive invariant: size_bytes must be non-negative.
ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_size_nonneg
    CHECK (size_bytes >= 0);
