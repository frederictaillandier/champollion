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

/** The card last rated or flagged, with that rating or flag, to take it back. */
@Serializable
data class Last(val card: Card, val review: String? = null, val flag: Flag? = null)

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
    /** Whether the words are not spoken when their card shows. */
    val muted: Boolean = false,
    /** Whether the next sync replaces the cards with a new session. */
    val reload: Boolean = false,
    /** Cards due for review on the backend at the last sync, null if never known. */
    val dueAtSync: Int? = null,
    /** New cards on the backend at the last sync, null if never known. */
    val newAtSync: Int? = null,
    /**
     * The cards that were new when downloaded, while they are in the session
     * or their ratings and flags are not sent: they count off the new cards,
     * not the due ones.
     */
    val newCards: Set<Long> = emptySet(),
    /** Epoch millis of the last successful sync, 0 if never. */
    val lastSync: Long = 0,
    /** Why the last sync failed, null if it worked. */
    val syncError: String? = null,
    /** The last rating or flag, while it can be taken back. */
    val last: Last? = null,
) {
    /** The card shown: the first in the queue, which is in random order. */
    fun current(): Card? =
        queue.firstOrNull()?.let { q -> cards.find { it.id == q.cardId } }

    /**
     * Cards due for review now, as far as the phone knows: those due on the
     * backend at the last sync, but those succeeded or flagged here since. A
     * failed card is still due.
     */
    fun due(): Int? = dueAtSync?.let { due ->
        val done = pending.count { it.rating == Rating.Succeeded.wire && it.cardId !in newCards } +
            flags.count { it.cardId !in newCards }
        maxOf(0, due - done)
    }

    /**
     * New cards, as far as the phone knows: those new on the backend at the
     * last sync, but those rated or flagged here since.
     */
    fun new(): Int? = newAtSync?.let { new ->
        val seen = (pending.map { it.cardId } + flags.map { it.cardId }).toSet() intersect newCards
        maxOf(0, new - seen.size)
    }

    fun flip(): Session = copy(flipped = !flipped)

    fun toggleMute(): Session = copy(muted = !muted)

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
            last = Last(card, review = review.id),
        )
    }

    /** Takes the current card out of the session, to be flagged. */
    fun flag(now: Long): Session {
        val card = current() ?: return this
        val flag = Flag(card.id, Instant.ofEpochMilli(now).toString())
        return copy(
            cards = cards.filterNot { it.id == card.id },
            queue = queue.filterNot { it.cardId == card.id },
            flipped = false,
            flags = flags + flag,
            last = Last(card, flag = flag),
        )
    }

    /** Whether the last rating or flag is not sent yet, so can be taken back. */
    fun canUndo(): Boolean {
        val last = last ?: return false
        return pending.any { it.id == last.review } || last.flag in flags
    }

    /**
     * Takes back the last rating or flag (a misclick): its card shows again
     * first, on its back, to be rated again.
     */
    fun undo(): Session {
        if (!canUndo()) return this
        val last = last ?: return this
        val card = last.card
        return copy(
            cards = cards.filterNot { it.id == card.id } + card,
            queue = listOf(Queued(card.id)) + queue.filterNot { it.cardId == card.id },
            flipped = true,
            pending = pending.filterNot { it.id == last.review },
            flags = flags.filterNot { it == last.flag },
            last = null,
        )
    }

    /**
     * Forgets the session's cards, so the next ones are downloaded again
     * (e.g. with better definitions). Ratings and flags not sent are kept.
     */
    fun restart(): Session =
        copy(cards = emptyList(), queue = emptyList(), flipped = false, reload = false)
            .withoutGoneNewCards()

    /** Forgets reviews the backend has received. */
    fun sent(reviews: List<Review>): Session {
        val ids = reviews.map { it.id }.toSet()
        return copy(
            pending = pending.filterNot { it.id in ids },
            last = last?.takeUnless { it.review in ids },
        ).withoutGoneNewCards()
    }

    /** Forgets flags the backend has received. */
    fun flagsSent(sent: List<Flag>): Session = copy(
        flags = flags - sent.toSet(),
        last = last?.takeUnless { it.flag in sent },
    ).withoutGoneNewCards()

    /** Forgets the new cards no longer in the session nor waiting to be sent. */
    private fun withoutGoneNewCards(): Session {
        val kept = cards.map { it.id } + pending.map { it.cardId } + flags.map { it.cardId }
        return copy(newCards = newCards intersect kept.toSet())
    }

    /**
     * Adds cards downloaded from the backend, except those already in the
     * session, or rated or flagged here and not sent yet.
     */
    fun merge(downloaded: List<Card>): Session {
        val known = queue.map { it.cardId }.toSet() + pending.map { it.cardId } +
            flags.map { it.cardId }
        val added = downloaded.filter { it.id !in known }
        val updated = cards.map { c -> downloaded.find { it.id == c.id } ?: c }
        return copy(
            cards = updated + added,
            queue = queue + added.map { Queued(it.id) },
            newCards = newCards + added.filter { it.isNew }.map { it.id },
        )
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
