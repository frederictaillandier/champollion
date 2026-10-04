CREATE TABLE words (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    lang text NOT NULL,
    text text NOT NULL,
    sentence text NOT NULL,
    game text NOT NULL,
    video text NOT NULL,
    seconds double precision NOT NULL,
    frame text NOT NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (lang, text)
);

-- Spaced repetition state of each word (SM-2, as in Anki).
CREATE TABLE cards (
    word_id bigint PRIMARY KEY REFERENCES words ON DELETE CASCADE,
    due timestamptz NOT NULL DEFAULT now(),
    interval_days double precision NOT NULL DEFAULT 0,
    ease double precision NOT NULL DEFAULT 2.5,
    reps integer NOT NULL DEFAULT 0,
    lapses integer NOT NULL DEFAULT 0
);
CREATE INDEX cards_due ON cards (due);

CREATE TABLE reviews (
    id uuid PRIMARY KEY,
    word_id bigint NOT NULL REFERENCES words ON DELETE CASCADE,
    rating text NOT NULL,
    reviewed_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);
