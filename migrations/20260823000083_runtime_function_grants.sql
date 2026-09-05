-- migrations/20260823000083_runtime_function_grants.sql
--
-- Hardening pass closing three classes of gaps found in review:
--
-- 1. FUNCTION EXECUTE grants for the production runtime role. Migrations
--    20260508000005 and 20260508000006 granted EXECUTE on their SECURITY
--    DEFINER lookup helpers only to the bootstrap role `aulalite`; the
--    hardened runtime role `aulalite_app` (20260517000020_app_role.sql) was
--    never granted, so under the documented production configuration
--    enrollment-code redemption and invitation acceptance fail with
--    "permission denied for function". Every later SECURITY DEFINER helper
--    follows a dual-grant pattern; this aligns the two early ones.
--
-- 2. verify_certificate hardening. The public certificate-verification
--    helper (20260610000036_certificates.sql) never revoked PUBLIC EXECUTE,
--    unlike every other definer helper. Revoke PUBLIC, then follow the same
--    dual-grant pattern used by the rest of the chain.
--
-- 3. Data integrity + index gaps:
--      * submissions.numeric_grade had no lower bound — negative grades were
--        storable by any tenant session. Added as NOT VALID + VALIDATE so
--        existing rows are checked without an ACCESS EXCLUSIVE lock on
--        rewrite; if legacy data violates it, VALIDATE fails loudly rather
--        than silently blessing bad data.
--      * assignment_categories.weight_percent documented as 0..=100 but
--        unconstrained; a negative weight corrupts weighted-grade
--        normalization. Same NOT VALID + VALIDATE treatment.
--      * lessons.course_id had no btree (only (module_id, sort_order)),
--        forcing sequential scans on every course-scoped lesson query.

-- ---------------------------------------------------------------------------
-- 1+2. Runtime-role EXECUTE grants (idempotent; safe to re-run).
-- ---------------------------------------------------------------------------

GRANT EXECUTE ON FUNCTION lookup_enrollment_code(TEXT) TO aulalite_app;
GRANT EXECUTE ON FUNCTION lookup_invitation_by_token(TEXT) TO aulalite_app;

REVOKE ALL ON FUNCTION verify_certificate(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION verify_certificate(TEXT) TO aulalite;
GRANT EXECUTE ON FUNCTION verify_certificate(TEXT) TO aulalite_app;

-- ---------------------------------------------------------------------------
-- 3a. lessons(course_id) index — course-scoped lesson listing, duplicate
--     course, progress rollups and search joins all filter on it.
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS lessons_course_id_idx ON lessons (course_id);

-- ---------------------------------------------------------------------------
-- 3b. Integrity constraints (NOT VALID first so no full-table lock, then
--     VALIDATE to prove existing rows conform).
-- ---------------------------------------------------------------------------

ALTER TABLE submissions
    DROP CONSTRAINT IF EXISTS submissions_numeric_grade_nonnegative;

ALTER TABLE submissions
    ADD CONSTRAINT submissions_numeric_grade_nonnegative
    CHECK (numeric_grade IS NULL OR numeric_grade >= 0)
    NOT VALID;

ALTER TABLE submissions
    VALIDATE CONSTRAINT submissions_numeric_grade_nonnegative;

ALTER TABLE assignment_categories
    DROP CONSTRAINT IF EXISTS assignment_categories_weight_percent_range;

ALTER TABLE assignment_categories
    ADD CONSTRAINT assignment_categories_weight_percent_range
    CHECK (weight_percent BETWEEN 0 AND 100)
    NOT VALID;

ALTER TABLE assignment_categories
    VALIDATE CONSTRAINT assignment_categories_weight_percent_range;
