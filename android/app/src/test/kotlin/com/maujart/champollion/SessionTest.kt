package com.maujart.champollion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
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
    fun ratingGoodMovesToTheNextCard() {
        val session = Session().merge(listOf(card(1), card(2))).rate(Rating.Good, now)
        assertEquals(2L, session.current(now)?.id)
        assertEquals(listOf(1L), session.pending.map { it.cardId })
        assertEquals("good", session.pending.single().rating)
    }

    @Test
    fun againShowsTheCardAgainTenMinutesLater() {
        val session = Session().merge(listOf(card(1))).rate(Rating.Again, now)
        assertNull(session.current(now))
        assertEquals(now + Session.AGAIN_DELAY_MS, session.nextAt(now))
        assertEquals(1L, session.current(now + Session.AGAIN_DELAY_MS)?.id)
    }

    @Test
    fun ratingFlipsTheNextCardToItsFront() {
        val session = Session().merge(listOf(card(1), card(2))).flip().rate(Rating.Easy, now)
        assertEquals(false, session.flipped)
    }

    @Test
    fun downloadsSkipCardsAlreadyInTheSessionOrRatedButNotSent() {
        val session = Session().merge(listOf(card(1), card(2))).rate(Rating.Good, now)
            .merge(listOf(card(1), card(2), card(3)))
        assertEquals(listOf(2L, 3L), session.queue.map { it.cardId })
    }

    @Test
    fun sentReviewsAreForgottenButNotNewerOnes() {
        val rated = Session().merge(listOf(card(1), card(2))).rate(Rating.Good, now)
        val sending = rated.pending
        val session = rated.rate(Rating.Hard, now).sent(sending)
        assertEquals(listOf(2L), session.pending.map { it.cardId })
    }

    @Test
    fun roundTripsThroughJson() {
        val session = Session().merge(listOf(card(1))).rate(Rating.Again, now)
        assertEquals(session, json.decodeFromString<Session>(json.encodeToString(session)))
    }
}
