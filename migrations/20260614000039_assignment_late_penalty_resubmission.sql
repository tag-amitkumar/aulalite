-- Late-submission penalty + resubmission policy.
-- Backward-compatible ADD COLUMN with safe defaults: existing rows get the
-- no-penalty / no-resubmission behaviour they had before.

ALTER TABLE assignments
    ADD COLUMN late_penalty_percent INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN max_resubmissions INTEGER NOT NULL DEFAULT 0;

ALTER TABLE assignments
    ADD CONSTRAINT assignments_late_penalty_percent_range
        CHECK (late_penalty_percent BETWEEN 0 AND 100),
    ADD CONSTRAINT assignments_max_resubmissions_nonneg
        CHECK (max_resubmissions >= 0);

-- attempt_number: 1 for the first submission, incremented on each
-- return-for-resubmit. applied_late_penalty_percent: the penalty actually
-- deducted at grade time (NULL until graded).
ALTER TABLE submissions
    ADD COLUMN attempt_number INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN applied_late_penalty_percent INTEGER;

ALTER TABLE submissions
    ADD CONSTRAINT submissions_attempt_number_positive
        CHECK (attempt_number >= 1),
    ADD CONSTRAINT submissions_applied_late_penalty_range
        CHECK (applied_late_penalty_percent IS NULL
               OR applied_late_penalty_percent BETWEEN 0 AND 100);
