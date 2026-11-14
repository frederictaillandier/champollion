package com.maujart.champollion

import kotlin.random.Random

/**
 * Level 3: the word is typed, from its translation. The answer shows as one
 * underscore per letter, one of them given; what isn't a letter (the space
 * of `připravit se`) is given too, and needn't be typed.
 */
object Typing {
    /** Cards from this level on are typed. */
    const val LEVEL = 3

    fun isTyped(card: Card): Boolean = card.level >= LEVEL

    /** Index, in [answer], of the letter given, chosen from [seed]. */
    fun hintIndex(answer: String, seed: Long): Int {
        val letters = answer.indices.filter { answer[it].isLetter() }
        if (letters.isEmpty()) return -1
        return letters[Random(seed).nextInt(letters.size)]
    }

    /**
     * The answer as typed so far: letters typed fill its letters in order,
     * the given letter shows until a letter is typed there, the rest are
     * underscores. Letters are spaced out, words further apart.
     */
    fun pattern(answer: String, hint: Int, typed: String): String {
        val letters = letters(typed)
        var next = 0
        return answer.mapIndexed { i, c ->
            when {
                c.isWhitespace() -> " "
                !c.isLetter() -> c.toString()
                else -> {
                    val shown = letters.getOrNull(next)?.toString()
                        ?: if (i == hint) c.toString() else "_"
                    next++
                    shown
                }
            }
        }.joinToString(" ")
    }

    /**
     * Whether [typed] is the answer: the same letters, accents included,
     * whatever their case and whether the spaces were typed.
     */
    fun isExact(typed: String, answer: String): Boolean =
        letters(typed).lowercase() == letters(answer).lowercase()

    private fun letters(text: String): String = text.filter { it.isLetter() }
}
