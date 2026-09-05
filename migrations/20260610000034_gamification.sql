-- Gamification (learning-suite Cycle 4): XP events with idempotent awards,
-- per-learner rollup stats (streaks), an achievement catalog with per-user
-- unlocks, and per-user leaderboard opt-out.
CREATE TABLE xp_events (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    course_id UUID REFERENCES courses(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    points INTEGER NOT NULL CHECK (points > 0),
    -- Idempotency: one award per logical action (e.g. lesson:user pair).
    dedup_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX xp_events_user_idx ON xp_events(user_id, created_at DESC);
CREATE INDEX xp_events_course_user_idx ON xp_events(course_id, user_id)
    WHERE course_id IS NOT NULL;

CREATE TABLE learner_stats (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    total_xp BIGINT NOT NULL DEFAULT 0,
    current_streak_days INTEGER NOT NULL DEFAULT 0,
    longest_streak_days INTEGER NOT NULL DEFAULT 0,
    last_activity_date DATE,
    PRIMARY KEY (tenant_id, user_id)
);

-- Catalog is global (not tenant-scoped); unlocks are per user.
CREATE TABLE achievements (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL
);

INSERT INTO achievements (id, title, description) VALUES
    ('first_lesson',  'First steps',   'Completed your first lesson.'),
    ('first_quiz',    'Quiz rookie',   'Submitted your first quiz.'),
    ('perfect_quiz',  'Perfectionist', 'Scored 100% on a quiz.'),
    ('streak_7',      'On fire',       'Learned 7 days in a row.'),
    ('xp_1000',       'Scholar',       'Earned 1,000 XP.');

CREATE TABLE achievement_unlocks (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    achievement_id TEXT NOT NULL REFERENCES achievements(id) ON DELETE CASCADE,
    unlocked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    seen BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (user_id, achievement_id)
);

CREATE TABLE leaderboard_opt_outs (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (tenant_id, user_id)
);

ALTER TABLE xp_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE xp_events FORCE ROW LEVEL SECURITY;
ALTER TABLE learner_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE learner_stats FORCE ROW LEVEL SECURITY;
ALTER TABLE achievement_unlocks ENABLE ROW LEVEL SECURITY;
ALTER TABLE achievement_unlocks FORCE ROW LEVEL SECURITY;
ALTER TABLE leaderboard_opt_outs ENABLE ROW LEVEL SECURITY;
ALTER TABLE leaderboard_opt_outs FORCE ROW LEVEL SECURITY;

CREATE POLICY xp_events_tenant_isolation ON xp_events
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY learner_stats_tenant_isolation ON learner_stats
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY achievement_unlocks_tenant_isolation ON achievement_unlocks
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY leaderboard_opt_outs_tenant_isolation ON leaderboard_opt_outs
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
