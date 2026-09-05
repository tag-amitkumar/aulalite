-- migrations/20260517000019_live_sessions_one_live_per_course.sql
-- Enforce "at most one live session per course" at the database level.
-- Prevents race conditions in start-now flows where two simultaneous
-- requests both pass an application-level "is there a live row?" check.
CREATE UNIQUE INDEX live_sessions_one_live_per_course
    ON live_sessions (course_id)
    WHERE status = 'live';
