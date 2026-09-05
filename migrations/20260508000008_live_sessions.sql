-- migrations/20260508000008_live_sessions.sql
CREATE TABLE live_sessions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    series_id UUID NOT NULL REFERENCES live_session_series(id) ON DELETE CASCADE,
    occurrence_index INTEGER NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'scheduled'
        CHECK (status IN ('scheduled','live','ended','cancelled')),
    starts_at TIMESTAMPTZ NOT NULL,
    duration_minutes INTEGER NOT NULL
        CHECK (duration_minutes BETWEEN 5 AND 480),
    actual_started_at TIMESTAMPTZ,
    actual_ended_at TIMESTAMPTZ,
    primary_teacher_id UUID NOT NULL REFERENCES users(id),
    ta_user_ids UUID[] NOT NULL DEFAULT '{}',
    mode TEXT NOT NULL DEFAULT 'lecture'
        CHECK (mode IN ('lecture','discussion')),
    recording_enabled BOOLEAN NOT NULL,
    main_path TEXT,
    hls_fallback_enabled BOOLEAN NOT NULL DEFAULT false,
    diverged BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (series_id, occurrence_index)
);

CREATE INDEX live_sessions_course_starts_idx ON live_sessions(course_id, starts_at);
CREATE INDEX live_sessions_series_idx ON live_sessions(series_id);

ALTER TABLE live_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_sessions FORCE ROW LEVEL SECURITY;

CREATE POLICY live_sessions_tenant_isolation ON live_sessions
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Now that live_sessions exists, add the deferred FK from lessons.
ALTER TABLE lessons
    ADD CONSTRAINT lessons_live_session_id_fkey
    FOREIGN KEY (live_session_id) REFERENCES live_sessions(id) ON DELETE SET NULL;
