-- Cards flagged on the phone as wrong (bad translation, OCR garbage...):
-- no longer reviewed, kept to be fixed or deleted.
ALTER TABLE cards ADD COLUMN flagged_at timestamptz;
