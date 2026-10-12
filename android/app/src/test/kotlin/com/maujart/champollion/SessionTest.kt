package com.maujart.champollion

import org.junit.Assert.assertEquals
import kotlin.random.Random
import org.junit.Test

class SessionTest {
    private fun card(id: Long) = Card(
        id = id,
        lang = "ces",
        lemma = "slovo$id",
        pos = "noun",
        gender = "n",
        translation = "word",
        due = "2026-10-04T12:00:00Z",
        reps = 0,
        sightings = emptyList(),
    )

    private val now = 1_000_000L

    @Test
    fun succeedingMovesToTheNextCard() {
        val session = Session().merge(listOf(card(1), card(2))).rate(Rating.Succeeded, now)
        assertEquals(2L, session.current()?.id)
        assertEquals(listOf(1L), session.pending.map { it.cardId })
        assertEquals("succeeded", session.pending.single().rating)
    }

    @Test
    fun aFailedCardGoesBackInTheQueueButNotFirst() {
        repeat(20) { seed ->
            val session = Session().merge(listOf(card(1), card(2), card(3)))
                .rate(Rating.Failed, now, Random(seed))
            assertEquals(2L, session.current()?.id)
            assertEquals(setOf(1L, 2L, 3L), session.queue.map { it.cardId }.toSet())
            assertEquals("failed", session.pending.single().rating)
        }
    }

    @Test
    fun theLastCardFailedIsShownAgainAtOnce() {
        val session = Session().merge(listOf(card(1))).rate(Rating.Failed, now)
        assertEquals(1L, session.current()?.id)
    }

    @Test
    fun ratingFlipsTheNextCardToItsFront() {
        val session = Session().merge(listOf(card(1), card(2))).flip().rate(Rating.Succeeded, now)
        assertEquals(false, session.flipped)
    }

    @Test
    fun stayingMutedAcrossCardsAndNewSessions() {
        val session = Session().merge(listOf(card(1), card(2))).toggleMute()
            .rate(Rating.Succeeded, now).restart().merge(listOf(card(3)))
        assertEquals(true, session.muted)
        assertEquals(false, session.toggleMute().muted)
    }

    @Test
    fun downloadsSkipCardsAlreadyInTheSessionOrRatedButNotSent() {
        val session = Session().merge(listOf(card(1), card(2))).rate(Rating.Succeeded, now)
            .merge(listOf(card(1), card(2), card(3)))
        assertEquals(listOf(2L, 3L), session.queue.map { it.cardId })
    }

    @Test
    fun sentReviewsAreForgottenButNotNewerOnes() {
        val rated = Session().merge(listOf(card(1), card(2))).rate(Rating.Succeeded, now)
        val sending = rated.pending
        val session = rated.rate(Rating.Failed, now).sent(sending)
        assertEquals(listOf(2L), session.pending.map { it.cardId })
    }

    @Test
    fun dueCountsOffCardsSucceededOrFlaggedButNotFailed() {
        val session = Session(dueAtSync = 10).merge(listOf(card(1), card(2), card(3)))
            .rate(Rating.Succeeded, now).rate(Rating.Failed, now).flag(now)
        assertEquals(8, session.due())
        assertEquals(null, Session().due())
    }

    @Test
    fun undoingASuccessShowsTheCardAgainOnItsBack() {
        val rated = Session(dueAtSync = 10).merge(listOf(card(1), card(2))).rate(Rating.Succeeded, now)
        assertEquals(true, rated.canUndo())
        val session = rated.undo()
        assertEquals(1L, session.current()?.id)
        assertEquals(listOf(1L, 2L), session.queue.map { it.cardId })
        assertEquals(true, session.flipped)
        assertEquals(emptyList<Review>(), session.pending)
        assertEquals(10, session.due())
        assertEquals(false, session.canUndo())
    }

    @Test
    fun undoingAFailureTakesTheCardOutOfTheQueueOnce() {
        val session = Session().merge(listOf(card(1), card(2), card(3)))
            .rate(Rating.Failed, now).undo()
        assertEquals(1L, session.current()?.id)
        assertEquals(listOf(1L, 2L, 3L), session.queue.map { it.cardId }.sorted())
        assertEquals(emptyList<Review>(), session.pending)
    }

    @Test
    fun undoingOnlyTakesBackTheLastRating() {
        val session = Session().merge(listOf(card(1), card(2), card(3)))
            .rate(Rating.Succeeded, now).rate(Rating.Succeeded, now).undo()
        assertEquals(2L, session.current()?.id)
        assertEquals(listOf(1L), session.pending.map { it.cardId })
        assertEquals(session, session.undo())
    }

    @Test
    fun undoingAFlagShowsTheCardAgain() {
        val session = Session().merge(listOf(card(1), card(2))).flag(now).undo()
        assertEquals(1L, session.current()?.id)
        assertEquals(emptyList<Flag>(), session.flags)
    }

    @Test
    fun theLastCardOfASessionCanBeTakenBack() {
        val session = Session().merge(listOf(card(1))).rate(Rating.Succeeded, now).undo()
        assertEquals(1L, session.current()?.id)
    }

    @Test
    fun aRatingOrFlagSentCannotBeTakenBack() {
        val rated = Session().merge(listOf(card(1), card(2))).rate(Rating.Succeeded, now)
        assertEquals(false, rated.sent(rated.pending).canUndo())
        val flagged = Session().merge(listOf(card(1), card(2))).flag(now)
        assertEquals(false, flagged.flagsSent(flagged.flags).canUndo())
    }

    @Test
    fun readsCardsWithAndWithoutALevel() {
        val old = """{"id":1,"lang":"ces","lemma":"rána","pos":"noun","gender":"f",
            "translation":"blow","due":"2026-10-04T12:00:00Z","reps":0,"sightings":[]}"""
        assertEquals(1, json.decodeFromString<Card>(old).level)
        val leveled = old.replace("\"reps\":0", "\"reps\":7,\"level\":2,\"form\":" +
            "{\"level\":2,\"form\":\"ránu\",\"translation\":\"blow\"}")
        assertEquals(CardForm(2, "ránu", "blow"), json.decodeFromString<Card>(leveled).form)
    }

    @Test
    fun readsASessionSavedWithTheOldDelays() {
        val saved = """{"queue":[{"cardId":1,"showAfter":1600000}]}"""
        assertEquals(listOf(Queued(1)), json.decodeFromString<Session>(saved).queue)
    }

    @Test
    fun roundTripsThroughJson() {
        val session = Session().merge(listOf(card(1))).rate(Rating.Failed, now)
        assertEquals(session, json.decodeFromString<Session>(json.encodeToString(session)))
    }
}
