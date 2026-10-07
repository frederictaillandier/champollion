package com.maujart.champollion

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/**
 * How the session's words sound, and the forms of the cards above level 1,
 * spoken by ElevenLabs on the backend. Kept on the phone with the session, so
 * they play offline.
 */
object Pronunciations {
    /**
     * Longest a word plays. A tap's action must end quickly, and Android may
     * freeze the app once it has, cutting the sound: the word is played
     * before it ends.
     */
    private const val MAX_PLAY_MS = 5_000L

    private fun dir(context: Context) = File(context.filesDir, "pronunciations")

    /** `12.mp3` for the word of card 12, `12-2.mp3` for its form of level 2. */
    private fun file(context: Context, cardId: Long, level: Int) =
        File(dir(context), if (level == 1) "$cardId.mp3" else "$cardId-$level.mp3")

    /**
     * Downloads the missing pronunciations of [cards], and deletes those of
     * cards no longer in the session. A card the backend has none for yet is
     * asked again at the next sync.
     */
    suspend fun download(context: Context, cards: List<Card>) {
        val dir = dir(context).apply { mkdirs() }
        val ids = cards.map { it.id }.toSet()
        dir.listFiles()
            ?.filter { it.nameWithoutExtension.substringBefore('-').toLongOrNull() !in ids }
            ?.forEach { it.delete() }
        for (card in cards) {
            for (level in listOfNotNull(1, card.form?.level)) {
                val file = file(context, card.id, level)
                if (file.exists()) continue
                val mp3 = Backend.audio(card.id, level) ?: continue
                // Written aside first, so a file found is always whole.
                val partial = File(dir, "${file.name}.part")
                partial.writeBytes(mp3)
                partial.renameTo(file)
            }
        }
    }

    /** Deletes every word downloaded. */
    fun clear(context: Context) {
        dir(context).listFiles()?.forEach { it.delete() }
    }

    /** Whether the card's word, or its form of [level], is on the phone. */
    fun has(context: Context, cardId: Long, level: Int = 1): Boolean =
        file(context, cardId, level).exists()

    /**
     * Plays the card's word, or its form of [level], if downloaded, and
     * returns once it ends.
     */
    suspend fun play(context: Context, card: Card, level: Int = 1) {
        val file = file(context, card.id, level).takeIf { it.exists() } ?: return
        val player = MediaPlayer()
        try {
            player.setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            player.setDataSource(file.path)
            player.prepare()
            withTimeoutOrNull(MAX_PLAY_MS) {
                suspendCancellableCoroutine<Unit> { done ->
                    player.setOnCompletionListener { if (done.isActive) done.resume(Unit) }
                    player.setOnErrorListener { _, _, _ ->
                        if (done.isActive) done.resume(Unit)
                        true
                    }
                    player.start()
                }
            }
        } catch (e: IOException) {
            // A broken file: downloaded again at the next sync.
            file.delete()
        } finally {
            player.release()
        }
    }
}
