-- First-run guided tour (learning-suite Cycle 7): per-user dismissed flag so
-- the teacher/admin spotlight tour shows exactly once per account.
ALTER TABLE users ADD COLUMN tour_dismissed BOOLEAN NOT NULL DEFAULT FALSE;
