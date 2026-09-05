-- migrations/20260509000018_file_assets_assignments.sql
-- Phase 1c: extend the linked_entity_type CHECK to allow attachment links
-- for assignments and submissions. Forward-only, mirrors 0015.

ALTER TABLE file_assets
    DROP CONSTRAINT IF EXISTS file_assets_linked_entity_type_check;

ALTER TABLE file_assets
    ADD CONSTRAINT file_assets_linked_entity_type_check
    CHECK (linked_entity_type IS NULL
        OR linked_entity_type IN (
            'course', 'lesson', 'session_recording',
            'assignment_attachment', 'submission_attachment'
        ));
