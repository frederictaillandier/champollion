package com.maujart.champollion

import android.content.Context
import androidx.datastore.core.Serializer
import androidx.datastore.dataStore
import java.io.InputStream
import java.io.OutputStream
import java.time.Instant
import java.util.UUID
import kotlin.random.Random
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Serializable

/** A card waiting in the session. */
@Serializable
data class Queued(val cardId: Long)

/**
 * The cards downloaded for review, kept on the phone so reviewing works
 * offline, and the reviews not sent to the backend yet. Shared by all widgets.
 */
@Serializable
data class Session(
    val cards: List<Card> = emptyList(),
    val queue: List<Queued> = emptyList(),
    /** Whether the current card shows its back. */
    val flipped: Boolean = false,
    val pending: List<Review> = emptyList(),
    /** Cards flagged here, not sent yet. */
    val flags: List<Flag> = emptyList(),
    /** Whether the next sync replaces the cards with a new session. */
    val reload: Boolean = false,
    /** Epoch millis of the last successful sync, 0 if never. */
    val lastSync: Long = 0,
    /** Why the last sync failed, null if it worked. */
    val syncError: String? = null,
) {
    /** The card shown: the first in the queue, which is in random order. */
    fun current(): Card? =
        queue.firstOrNull()?.let { q -> cards.find { it.id == q.cardId } }

    fun flip(): Session = copy(flipped = !flipped)

    /**
     * Records a rating of the current card. A failed card goes back in the
     * queue at a random place, after the next card if there is one; a
     * succeeded one ends here, and the backend schedules its next review once
     * the rating is sent.
     */
    fun rate(rating: Rating, now: Long, random: Random = Random): Session {
        val card = current() ?: return this
        val queue = queue.filterNot { it.cardId == card.id }.toMutableList()
        if (rating == Rating.Failed) {
            queue.add(random.nextInt(minOf(1, queue.size), queue.size + 1), Queued(card.id))
        }
        val review = Review(
            id = UUID.randomUUID().toString(),
            cardId = card.id,
            rating = rating.wire,
            reviewedAt = Instant.ofEpochMilli(now).toString(),
        )
        return copy(
            cards = cards.filter { c -> queue.any { it.cardId == c.id } },
            queue = queue,
            flipped = false,
            pending = pending + review,
        )
    }

    /** Takes the current card out of the session, to be flagged. */
    fun flag(now: Long): Session {
        val card = current() ?: return this
        return copy(
            cards = cards.filterNot { it.id == card.id },
            queue = queue.filterNot { it.cardId == card.id },
            flipped = false,
            flags = flags + Flag(card.id, Instant.ofEpochMilli(now).toString()),
        )
    }

    /**
     * Forgets the session's cards, so the next ones are downloaded again
     * (e.g. with better definitions). Ratings and flags not sent are kept.
     */
    fun restart(): Session =
        copy(cards = emptyList(), queue = emptyList(), flipped = false, reload = false)

    /** Forgets reviews the backend has received. */
    fun sent(reviews: List<Review>): Session {
        val ids = reviews.map { it.id }.toSet()
        return copy(pending = pending.filterNot { it.id in ids })
    }

    /** Forgets flags the backend has received. */
    fun flagsSent(sent: List<Flag>): Session = copy(flags = flags - sent.toSet())

    /**
     * Adds cards downloaded from the backend, except those already in the
     * session, or rated or flagged here and not sent yet.
     */
    fun merge(downloaded: List<Card>): Session {
        val known = queue.map { it.cardId }.toSet() + pending.map { it.cardId } +
            flags.map { it.cardId }
        val added = downloaded.filter { it.id !in known }
        val updated = cards.map { c -> downloaded.find { it.id == c.id } ?: c }
        return copy(cards = updated + added, queue = queue + added.map { Queued(it.id) })
    }
}

object SessionSerializer : Serializer<Session> {
    override val defaultValue = Session()

    override suspend fun readFrom(input: InputStream): Session =
        try {
            json.decodeFromString(input.readBytes().decodeToString())
        } catch (e: SerializationException) {
            defaultValue
        }

    override suspend fun writeTo(t: Session, output: OutputStream) {
        output.write(json.encodeToString(t).toByteArray())
    }
}

const val SESSION_FILE = "session.json"

val Context.session by dataStore(SESSION_FILE, SessionSerializer)
