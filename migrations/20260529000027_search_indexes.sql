-- migrations/20260529000027_search_indexes.sql
-- Structured search support: GIN trigram indexes for the /v1/search endpoint.
-- pg_trgm is already enabled (see 20260503000001_extensions.sql). These
-- indexes accelerate the ILIKE '%q%' / `% q` similarity predicates the
-- search queries use. No new tables or RLS — search reuses the existing
-- tenant isolation + course-visibility rules at query time.

CREATE INDEX IF NOT EXISTS courses_title_trgm
    ON courses USING gin (title gin_trgm_ops);
CREATE INDEX IF NOT EXISTS courses_description_trgm
    ON courses USING gin (description gin_trgm_ops);

CREATE INDEX IF NOT EXISTS assignments_title_trgm
    ON assignments USING gin (title gin_trgm_ops);
CREATE INDEX IF NOT EXISTS assignments_instructions_md_trgm
    ON assignments USING gin (instructions_md gin_trgm_ops);
