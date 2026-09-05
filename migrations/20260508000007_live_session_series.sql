-- migrations/20260508000007_live_session_series.sql
CREATE TABLE live_session_series (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    duration_minutes INTEGER NOT NULL
        CHECK (duration_minutes BETWEEN 5 AND 480),
    frequency TEXT NOT NULL
        CHECK (frequency IN ('none','daily','weekly','biweekly','monthly')),
    byweekday TEXT[],
    end_kind TEXT NOT NULL
        CHECK (end_kind IN ('count','until','open')),
    occurrence_count INTEGER,
    end_until TIMESTAMPTZ,
    primary_teacher_id UUID NOT NULL REFERENCES users(id),
    recording_enabled BOOLEAN,
    open_cursor TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CHECK (
        (end_kind = 'count' AND occurrence_count IS NOT NULL AND end_until IS NULL) OR
        (end_kind = 'until' AND end_until IS NOT NULL AND occurrence_count IS NULL) OR
        (end_kind = 'open'  AND occurrence_count IS NULL AND end_until IS NULL)
    ),

    CHECK (
        (frequency IN ('weekly','biweekly')
            AND byweekday IS NOT NULL
            AND array_length(byweekday, 1) > 0) OR
        (frequency NOT IN ('weekly','biweekly')
            AND byweekday IS NULL)
    ),

    CONSTRAINT live_session_series_open_cursor_check
        CHECK (open_cursor IS NULL OR end_kind = 'open')
);

CREATE INDEX live_session_series_course_idx ON live_session_series(course_id);

ALTER TABLE live_session_series ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_session_series FORCE ROW LEVEL SECURITY;

CREATE POLICY live_session_series_tenant_isolation ON live_session_series
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
