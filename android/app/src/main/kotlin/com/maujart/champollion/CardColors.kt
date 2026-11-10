package com.maujart.champollion

import androidx.compose.ui.graphics.Color
import androidx.glance.unit.ColorProvider

/**
 * A card's colours: its part of speech's (nouns red, verbs green, adjectives
 * yellow, the rest grey), darker at each level.
 */
class CardColors(background: Color, text: Color, muted: Color) {
    val background = ColorProvider(background)
    val text = ColorProvider(text)
    val muted = ColorProvider(muted)

    companion object {
        private val red = listOf(0xFFFFCDD2, 0xFFE57373, 0xFFC62828)
        private val green = listOf(0xFFC8E6C9, 0xFF81C784, 0xFF2E7D32)
        private val yellow = listOf(0xFFFFF9C4, 0xFFFFD54F, 0xFFF9A825)
        private val grey = listOf(0xFFEEEEEE, 0xFFBDBDBD, 0xFF616161)

        fun of(card: Card): CardColors {
            val shades = when (card.pos) {
                "noun" -> red
                "verb" -> green
                "adjective" -> yellow
                else -> grey
            }
            // Levels past the last shade keep it.
            val shade = (card.level - 1).coerceIn(0, shades.lastIndex)
            val background = Color(shades[shade])
            // Dark text while the card is light, white once it is dark.
            return if (shade < 2) {
                CardColors(background, Color(0xDE000000), Color(0x99000000))
            } else {
                CardColors(background, Color.White, Color(0xCCFFFFFF))
            }
        }
    }
}
