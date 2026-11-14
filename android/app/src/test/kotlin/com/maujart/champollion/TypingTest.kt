package com.maujart.champollion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TypingTest {
    @Test
    fun givesOneLetterAndAnUnderscoreForEachOther() {
        // The hint is the č of útočit.
        assertEquals("_ _ _ č _ _", Typing.pattern("útočit", 3, ""))
    }

    @Test
    fun fillsTheLettersTypedInOrder() {
        assertEquals("ú t o č _ _", Typing.pattern("útočit", 3, "úto"))
        assertEquals("ú t o x _ _", Typing.pattern("útočit", 3, "útox"))
    }

    @Test
    fun givesTheSpaceOfTwoWords() {
        assertEquals("_ _ _ _ _ _ _ _ _   _ e", Typing.pattern("připravit se", 11, ""))
        assertEquals("p ř i p r a v i t   s _", Typing.pattern("připravit se", 0, "připravit s"))
    }

    @Test
    fun theHintIsALetterThatDependsOnTheSeed() {
        repeat(50) { seed ->
            val i = Typing.hintIndex("připravit se", seed.toLong())
            assertTrue("připravit se"[i].isLetter())
        }
        val hints = (0L until 50L).map { Typing.hintIndex("potřebovat", it) }.toSet()
        assertTrue(hints.size > 3)
    }

    @Test
    fun onlyTheExactWordWithItsAccentsSucceeds() {
        assertTrue(Typing.isExact("věž", "věž"))
        assertTrue(Typing.isExact("Věž ", "věž"))
        assertTrue(Typing.isExact("připravitse", "připravit se"))
        assertTrue(Typing.isExact("připravit se", "připravit se"))
        assertFalse(Typing.isExact("vez", "věž"))
        assertFalse(Typing.isExact("věže", "věž"))
        assertFalse(Typing.isExact("vě", "věž"))
    }

    @Test
    fun cardsFromLevelThreeAreTyped() {
        fun card(level: Int) = Card(
            id = 1, lang = "ces", lemma = "věž", pos = "noun", gender = "f",
            translation = "tower", due = "", reps = 3, level = level, sightings = emptyList(),
        )
        assertFalse(Typing.isTyped(card(2)))
        assertTrue(Typing.isTyped(card(3)))
        assertTrue(Typing.isTyped(card(4)))
    }
}
