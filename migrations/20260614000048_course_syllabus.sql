-- Add the student-readable syllabus + grading-policy markdown to courses.
-- Both nullable; courses created before this migration simply have NULLs.
-- The columns inherit the existing courses RLS policy + aulalite_app grants
-- (see 20260508000001_courses.sql / 20260517000020_app_role.sql), so no new
-- policy or GRANT is required.
ALTER TABLE courses ADD COLUMN syllabus_md TEXT;
ALTER TABLE courses ADD COLUMN grading_policy_md TEXT;
