package com.maujart.champollion

import java.io.FileNotFoundException
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// The JSON types of champollion-backend (api/src/lib.rs).

@Serializable
data class Card(
    val id: Long,
    val lang: String,
    val lemma: String,
    val pos: String,
    val gender: String,
    val translation: String,
    val due: String,
    val reps: Int,
    /** 1 for a new card; a card learnt well enough moves up to its next form's. */
    val level: Int = 1,
    /** The form learnt at the card's level, none at level 1. */
    val form: CardForm? = null,
    val sightings: List<CardSighting>,
)

/** A form of the card's word, e.g. `schovám se` ("I'll hide"). */
@Serializable
data class CardForm(
    val level: Int,
    val form: String,
    val translation: String,
)

@Serializable
data class CardSighting(
    val form: String,
    val sentence: String,
    @SerialName("sentence_translation") val sentenceTranslation: String,
    val definition: String,
)

/** Whether a card was remembered. */
enum class Rating(val label: String) {
    Failed("Failed"),
    Succeeded("Succeed");

    /** As the backend spells it. */
    val wire: String get() = name.lowercase()
}

@Serializable
data class Review(
    /** Chosen here, so sending a review twice applies it once. */
    val id: String,
    @SerialName("card_id") val cardId: Long,
    val rating: String,
    @SerialName("reviewed_at") val reviewedAt: String,
)

@Serializable
private data class DueCount(val due: Int)

@Serializable
private data class ReviewBatch(val reviews: List<Review>)

/** A card flagged as having an issue: the backend stops showing it. */
@Serializable
data class Flag(
    @SerialName("card_id") val cardId: Long,
    @SerialName("flagged_at") val flaggedAt: String,
)

@Serializable
private data class FlagBatch(val flags: List<Flag>)

val json = Json { ignoreUnknownKeys = true }

/** champollion-backend, through the WireGuard tunnel. */
object Backend {
    private const val TIMEOUT_MS = 15_000

    suspend fun dueCards(limit: Int): List<Card> =
        json.decodeFromString(request("GET", "/cards/due?limit=$limit", null).decodeToString())

    /** How many cards are due now on the backend, new ones included. */
    suspend fun dueCount(): Int =
        json.decodeFromString<DueCount>(request("GET", "/cards/due/count", null).decodeToString()).due

    /**
     * The MP3 of the card's word (level 1) or of its form of a level above,
     * or null while the backend has none.
     */
    suspend fun audio(cardId: Long, level: Int): ByteArray? =
        try {
            request("GET", "/cards/$cardId/audio?level=$level", null)
        } catch (e: FileNotFoundException) {
            null
        }

    suspend fun sendReviews(reviews: List<Review>) {
        request("POST", "/reviews", json.encodeToString(ReviewBatch(reviews)))
    }

    suspend fun sendFlags(flags: List<Flag>) {
        request("POST", "/flags", json.encodeToString(FlagBatch(flags)))
    }

    /** Throws [FileNotFoundException] on a 404. */
    private suspend fun request(method: String, path: String, body: String?): ByteArray =
        withContext(Dispatchers.IO) {
            val connection = URL(BuildConfig.BACKEND_URL + path).openConnection() as HttpURLConnection
            try {
                connection.requestMethod = method
                connection.connectTimeout = TIMEOUT_MS
                connection.readTimeout = TIMEOUT_MS
                if (body != null) {
                    connection.doOutput = true
                    connection.setRequestProperty("Content-Type", "application/json")
                    connection.outputStream.use { it.write(body.toByteArray()) }
                }
                when (connection.responseCode) {
                    in 200..299 -> {}
                    404 -> throw FileNotFoundException("$method $path: HTTP 404")
                    else -> throw IOException("$method $path: HTTP ${connection.responseCode}")
                }
                connection.inputStream.use { it.readBytes() }
            } finally {
                connection.disconnect()
            }
        }
}
