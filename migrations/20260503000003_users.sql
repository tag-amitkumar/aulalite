CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    firebase_uid TEXT NOT NULL UNIQUE,
    email CITEXT NOT NULL UNIQUE,
    display_name TEXT,
    avatar_url TEXT,
    locale TEXT,
    is_platform_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ
);

CREATE INDEX users_firebase_uid_idx ON users(firebase_uid);
CREATE INDEX users_email_idx ON users(email);
