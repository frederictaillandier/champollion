-- The phone rates a card failed or succeeded, no longer as in Anki.
UPDATE reviews SET rating = CASE rating WHEN 'again' THEN 'failed' ELSE 'succeeded' END
WHERE rating IN ('again', 'hard', 'good', 'easy');
