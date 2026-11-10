-- How each card's dictionary form sounds, spoken by ElevenLabs (MP3).
CREATE TABLE pronunciations (
    card_id bigint PRIMARY KEY REFERENCES cards ON DELETE CASCADE,
    audio bytea NOT NULL,
    -- ElevenLabs voice and model, to tell which to make again after a change.
    voice text NOT NULL,
    model text NOT NULL,
    made_at timestamptz NOT NULL DEFAULT now()
);
