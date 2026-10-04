package com.maujart.champollion

import android.content.Context
import androidx.datastore.core.Serializer
import androidx.datastore.dataStore
import java.io.InputStream
import java.io.OutputStream
import java.time.Instant
import java.util.UUID
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Serializable

/** A card waiting in the session, shown from [showAfter] (epoch millis). */
@Serializable
data class Queued(val cardId: Long, val showAfter: Long = 0)

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
    /** Epoch millis of the last successful sync, 0 if never. */
    val lastSync: Long = 0,
    /** Why the last sync failed, null if it worked. */
    val syncError: String? = null,
) {
    /** The first card whose time has come. */
    fun current(now: Long): Card? =
        queue.firstOrNull { it.showAfter <= now }?.let { q -> cards.find { it.id == q.cardId } }

    /** When the next card is ready, if none is now. */
    fun nextAt(now: Long): Long? =
        if (current(now) != null) null else queue.minOfOrNull { it.showAfter }

    fun flip(): Session = copy(flipped = !flipped)

    /**
     * Records a rating of the current card. "Again" shows it again in
     * [AGAIN_DELAY_MS], within this session; other ratings end it here, and
     * the backend schedules its next review once the rating is sent.
     */
    fun rate(rating: Rating, now: Long): Session {
        val card = current(now) ?: return this
        val queue = queue.filterNot { it.cardId == card.id }.toMutableList()
        if (rating == Rating.Again) {
            queue += Queued(card.id, now + AGAIN_DELAY_MS)
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

    /** Forgets reviews the backend has received. */
    fun sent(reviews: List<Review>): Session {
        val ids = reviews.map { it.id }.toSet()
        return copy(pending = pending.filterNot { it.id in ids })
    }

    /**
     * Adds cards downloaded from the backend, except those already in the
     * session or rated here and not sent yet.
     */
    fun merge(downloaded: List<Card>): Session {
        val known = queue.map { it.cardId }.toSet() + pending.map { it.cardId }
        val added = downloaded.filter { it.id !in known }
        val updated = cards.map { c -> downloaded.find { it.id == c.id } ?: c }
        return copy(cards = updated + added, queue = queue + added.map { Queued(it.id) })
    }

    companion object {
        const val AGAIN_DELAY_MS = 10 * 60 * 1000L
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
