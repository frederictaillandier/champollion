-- Cards become one per dictionary form, made by the daemon with Claude, and
-- each word read in the game becomes a sighting of its card. The tables of
-- 0001 only held test words: they are dropped, not converted.
DROP TABLE reviews, cards, words;

CREATE TABLE cards (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    lang text NOT NULL,
    lemma text NOT NULL,
    pos text NOT NULL,
    gender text NOT NULL,
    translation text NOT NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    -- Spaced repetition state (SM-2, as in Anki).
    due timestamptz NOT NULL DEFAULT now(),
    interval_days double precision NOT NULL DEFAULT 0,
    ease double precision NOT NULL DEFAULT 2.5,
    reps integer NOT NULL DEFAULT 0,
    lapses integer NOT NULL DEFAULT 0,
    UNIQUE (lang, lemma, pos)
);
CREATE INDEX cards_due ON cards (due);

CREATE TABLE sightings (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    card_id bigint NOT NULL REFERENCES cards ON DELETE CASCADE,
    form text NOT NULL,
    sentence text NOT NULL,
    sentence_translation text NOT NULL,
    definition text NOT NULL,
    game text NOT NULL,
    video text NOT NULL,
    seconds double precision NOT NULL,
    frame text NOT NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (card_id, form, sentence)
);

CREATE TABLE reviews (
    id uuid PRIMARY KEY,
    card_id bigint NOT NULL REFERENCES cards ON DELETE CASCADE,
    rating text NOT NULL,
    reviewed_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);
