-- migrations/20260517000021_live_sessions_one_live_per_user.sql
-- Enforce "at most one live session per user (primary_teacher_id)" at the
-- database level. Combined with live_sessions_one_live_per_course, this
-- guarantees a teacher cannot run two concurrent live sessions across
-- different courses, devices, or logins. Distinct users may still run
-- live sessions concurrently — the constraint scopes only by teacher.
CREATE UNIQUE INDEX live_sessions_one_live_per_user
    ON live_sessions (primary_teacher_id)
    WHERE status = 'live';
