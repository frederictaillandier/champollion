-- Levels: a card learnt well enough moves up to the level of its next form,
-- where its back teaches that form (e.g. the 1st person singular of a verb).
ALTER TABLE cards ADD COLUMN level smallint NOT NULL DEFAULT 1;

CREATE TABLE card_forms (
    card_id bigint NOT NULL REFERENCES cards ON DELETE CASCADE,
    level smallint NOT NULL CHECK (level > 1),
    form text NOT NULL,
    translation text NOT NULL,
    PRIMARY KEY (card_id, level)
);

-- A card has a pronunciation per level: its dictionary form at level 1,
-- then its form of each level.
ALTER TABLE pronunciations ADD COLUMN level smallint NOT NULL DEFAULT 1;
ALTER TABLE pronunciations DROP CONSTRAINT pronunciations_pkey;
ALTER TABLE pronunciations ADD PRIMARY KEY (card_id, level);
-- The text spoken, so a form or lemma changed since is spoken again.
ALTER TABLE pronunciations ADD COLUMN text text;
UPDATE pronunciations p SET text = c.lemma FROM cards c WHERE c.id = p.card_id;
ALTER TABLE pronunciations ALTER COLUMN text SET NOT NULL;
