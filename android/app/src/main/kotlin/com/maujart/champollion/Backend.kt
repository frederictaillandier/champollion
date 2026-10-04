package com.maujart.champollion

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
    val sightings: List<CardSighting>,
)

@Serializable
data class CardSighting(
    val form: String,
    val sentence: String,
    @SerialName("sentence_translation") val sentenceTranslation: String,
    val definition: String,
)

/** The backend also knows Hard and Good, which the widget leaves out. */
enum class Rating(val label: String) {
    Again("Again"),
    Easy("Easy");

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
        json.decodeFromString(request("GET", "/cards/due?limit=$limit", null))

    suspend fun sendReviews(reviews: List<Review>) {
        request("POST", "/reviews", json.encodeToString(ReviewBatch(reviews)))
    }

    suspend fun sendFlags(flags: List<Flag>) {
        request("POST", "/flags", json.encodeToString(FlagBatch(flags)))
    }

    private suspend fun request(method: String, path: String, body: String?): String =
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
                if (connection.responseCode !in 200..299) {
                    throw IOException("$method $path: HTTP ${connection.responseCode}")
                }
                connection.inputStream.use { it.readBytes().decodeToString() }
            } finally {
                connection.disconnect()
            }
        }
}
