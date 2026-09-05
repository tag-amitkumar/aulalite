-- migrations/20260509000015_file_assets_session_recording.sql
-- Extend the linked_entity_type check to allow 'session_recording' for Phase 1b-δ.

ALTER TABLE file_assets
    DROP CONSTRAINT IF EXISTS file_assets_linked_entity_type_check;

ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN ('course', 'lesson', 'session_recording'));
