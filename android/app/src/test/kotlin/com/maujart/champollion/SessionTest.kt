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
