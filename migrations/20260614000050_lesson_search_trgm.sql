-- GIN trigram indexes for the lesson branch of /v1/search (db::search::search_lessons).
-- pg_trgm is already enabled (20260503000001_extensions.sql); courses + assignments
-- title/body indexes already exist (20260529000027_search_indexes.sql). These add the
-- two missing lesson columns so the ILIKE '%q%' / `% q` predicates stay index-backed.
-- No new tables/RLS: lesson search reuses tenant isolation + course visibility at query time.

CREATE INDEX IF NOT EXISTS lessons_title_trgm
    ON lessons USING gin (title gin_trgm_ops);
CREATE INDEX IF NOT EXISTS lessons_body_md_trgm
    ON lessons USING gin (body_md gin_trgm_ops);
