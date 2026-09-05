-- migrations/20260823000085_course_self_enrollment.sql
--
-- Course catalog / self-enrollment. Teachers can flip a published course to
-- "open" so any active tenant member can discover it in the catalog and enroll
-- themselves — no code or invite required. Default OFF keeps the existing
-- code/invite/bulk enrollment model untouched; seat caps and RLS still apply
-- on every self-enrollment (same ensure_* paths the other flows use).

ALTER TABLE courses
    ADD COLUMN IF NOT EXISTS self_enrollment_enabled BOOLEAN NOT NULL DEFAULT false;
