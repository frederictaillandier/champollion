-- A sighting is a word read on a frame. Its sentence is now Claude's reading
-- of the screen, which can be made again better: it no longer identifies it.
ALTER TABLE sightings DROP CONSTRAINT sightings_card_id_form_sentence_key;
ALTER TABLE sightings ADD UNIQUE (card_id, form, frame);
