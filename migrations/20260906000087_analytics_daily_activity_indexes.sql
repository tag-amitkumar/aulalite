-- Analytics dashboard: sargable per-day indexes for daily_activity.
--
-- db::analytics::daily_activity walks one row per day (30-90 of them) and runs
-- four correlated count subqueries per day. The predicates used to read
--   tenant_id = $1 AND <timestamp_col>::date = d::date
-- and the cast on the indexed column defeated any btree seek, so every day
-- scanned every tenant row of every one of the four tables: cost grew as
-- days x tenant_rows x 4.
--
-- WHY THESE ARE PLAIN COLUMN INDEXES AND NOT EXPRESSION INDEXES
--
-- The obvious fix is an expression index matching the query,
-- `(tenant_id, (starts_at::date))`. Postgres rejects it:
--
--     ERROR: functions in index expression must be marked IMMUTABLE
--
-- All four columns are `timestamp with time zone`, and `timestamptz -> date`
-- depends on the session TimeZone, which makes the cast STABLE rather than
-- IMMUTABLE. Index expressions must be immutable, because the stored value
-- would otherwise silently disagree with the same expression evaluated later
-- under a different setting. That is not a quirk of this deployment -- the
-- statement cannot succeed on any database where these columns are timestamptz.
-- (An earlier attempt at exactly that index exists in an abandoned checkout;
-- it fails on every boot and takes the backend down with it, because a failing
-- migration aborts startup.)
--
-- So the index stays on the raw column and the QUERY moves to the equivalent
-- half-open range instead:
--
--     <col> >= d::date::timestamptz AND <col> < (d::date + 1)::timestamptz
--
-- That is the same bucketing -- `<col>::date = D` holds exactly when <col>
-- falls in [midnight D, midnight D+1) in the session time zone, and both forms
-- read that zone the same way -- but it is a range over the stored value, so a
-- plain btree can seek. Keeping the day boundary session-relative rather than
-- pinning it to UTC is deliberate: it preserves the existing behaviour of the
-- dashboard exactly, where an `AT TIME ZONE 'UTC'` expression index would have
-- quietly redefined what "a day" means.
--
-- tenant_id leads each index so the RLS-scoped equality is the prefix and the
-- timestamp range is the second column -- the shape a per-tenant, per-day count
-- can satisfy with a single range scan.

CREATE INDEX IF NOT EXISTS live_sessions_tenant_starts_at_idx
    ON live_sessions (tenant_id, starts_at);

CREATE INDEX IF NOT EXISTS attendance_tenant_first_joined_at_idx
    ON attendance (tenant_id, first_joined_at);

CREATE INDEX IF NOT EXISTS submissions_tenant_submitted_at_idx
    ON submissions (tenant_id, submitted_at);

CREATE INDEX IF NOT EXISTS lesson_completions_tenant_completed_at_idx
    ON lesson_completions (tenant_id, completed_at);
