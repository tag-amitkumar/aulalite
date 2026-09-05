-- Flashcards (learning-suite Cycle 6): teacher-authored decks per course with
-- per-student SM-2 review state (vocabulary mirrors kinetics ui-learn:
-- ease / interval_days / repetitions / Again|Hard|Good|Easy).
CREATE TABLE flashcard_decks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    course_id UUID NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published')),
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX flashcard_decks_course_idx ON flashcard_decks(course_id, status);

CREATE TABLE flashcards (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id UUID NOT NULL,
    deck_id UUID NOT NULL REFERENCES flashcard_decks(id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    front TEXT NOT NULL,
    back TEXT NOT NULL
);

CREATE INDEX flashcards_deck_idx ON flashcards(deck_id, position);

CREATE TABLE flashcard_review_state (
    tenant_id UUID NOT NULL,
    card_id UUID NOT NULL REFERENCES flashcards(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ease REAL NOT NULL DEFAULT 2.5,
    interval_days REAL NOT NULL DEFAULT 0,
    repetitions INTEGER NOT NULL DEFAULT 0,
    due_date DATE NOT NULL DEFAULT CURRENT_DATE,
    last_rating TEXT CHECK (last_rating IN ('again','hard','good','easy')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (card_id, user_id)
);

CREATE INDEX flashcard_review_due_idx ON flashcard_review_state(user_id, due_date);

ALTER TABLE flashcard_decks ENABLE ROW LEVEL SECURITY;
ALTER TABLE flashcard_decks FORCE ROW LEVEL SECURITY;
ALTER TABLE flashcards ENABLE ROW LEVEL SECURITY;
ALTER TABLE flashcards FORCE ROW LEVEL SECURITY;
ALTER TABLE flashcard_review_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE flashcard_review_state FORCE ROW LEVEL SECURITY;

CREATE POLICY flashcard_decks_tenant_isolation ON flashcard_decks
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY flashcards_tenant_isolation ON flashcards
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
CREATE POLICY flashcard_review_state_tenant_isolation ON flashcard_review_state
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
