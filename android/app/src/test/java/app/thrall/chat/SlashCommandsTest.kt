package app.thrall.chat

import app.thrall.protocol.CommandItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SlashCommandsTest {
    private val known = listOf(
        CommandItem("model", "Switch model", "[model id]"),
        CommandItem("mode", "Permission mode", "[mode]"),
        CommandItem("folder", "Folder", "[path]"),
        CommandItem("humanise", "Rewrite", "<text>", skill = true),
    )

    @Test
    fun commandsAreReadFromWhatWasTyped() {
        assertEquals(SlashCommands.Typed("model", "openai/gpt-6-luna"), SlashCommands.parse("/model openai/gpt-6-luna"))
        assertEquals(SlashCommands.Typed("status", ""), SlashCommands.parse("  /Status  "))
        assertEquals(SlashCommands.Typed("folder", "C:\\Work Stuff\\x"), SlashCommands.parse("/folder C:\\Work Stuff\\x"))
        assertNull(SlashCommands.parse("hello /model"))
        assertNull(SlashCommands.parse("/"))
        assertNull(SlashCommands.parse("/home/me/notes.md is there"))
    }

    @Test
    fun skillsAndUnknownNamesGoToThrall() {
        assertTrue(SlashCommands.runsOnEngine(SlashCommands.Typed("model", "x"), known))
        assertFalse(SlashCommands.runsOnEngine(SlashCommands.Typed("humanise", "x"), known))
        assertFalse(SlashCommands.runsOnEngine(SlashCommands.Typed("my-skill", ""), known))
    }

    @Test
    fun typingASlashOffersCommands() {
        assertEquals(listOf("model", "mode"), SlashCommands.suggestions("/mo", known).map { it.name })
        assertEquals(4, SlashCommands.suggestions("/", known).size)
        assertTrue(SlashCommands.suggestions("/model x", known).isEmpty())
        assertTrue(SlashCommands.suggestions("model", known).isEmpty())
    }
}
