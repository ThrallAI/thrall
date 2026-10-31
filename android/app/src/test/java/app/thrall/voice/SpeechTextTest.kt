package app.thrall.voice

import app.thrall.ui.Block
import app.thrall.ui.blocks
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeechTextTest {
    @Test
    fun markdownIsReadAsWords() {
        val md = """
            ## What I built
            Saved **three** files in `launch-content/` — see [the guide](https://example.com/guide).
            ```python
            print("not read")
            ```
            | File | What it is |
            |------|------------|
            | `README-public.md` | New README |
            - Keep dungeon_adventure.py and _this_ too
        """.trimIndent()
        assertEquals(
            listOf(
                "What I built",
                "Saved three files in launch-content/ — see the guide.",
                "File, What it is",
                "README-public.md, New README",
                "Keep dungeon_adventure.py and this too",
            ).joinToString("\n"),
            SpeechText.spoken(md),
        )
    }

    @Test
    fun longTextIsSplitAtSentences() {
        val text = "One two three. Four five six. Seven eight nine."
        val parts = SpeechText.pieces(text, 20)
        assertTrue(parts.all { it.length <= 20 })
        assertEquals(text, parts.joinToString(" "))
        assertEquals(listOf("short"), SpeechText.pieces("short", 20))
    }

    @Test
    fun saySlashForACommand() {
        assertEquals("/status", SpeechText.dictated("slash Status"))
        assertEquals("/folder C:\\Work", SpeechText.dictated("Forward slash folder C:\\Work"))
        assertEquals("make a slash command", SpeechText.dictated("make a slash command"))
    }

    @Test
    fun tablesAreBlocks() {
        val b = blocks("Before\n\n| A | B |\n|:--|--:|\n| 1 | 2 |\n| 3 |\n\nAfter")
        assertEquals(Block.Paragraph("Before"), b[0])
        assertEquals(Block.Table(listOf("A", "B"), listOf(listOf("1", "2"), listOf("3", ""))), b[1])
        assertEquals(Block.Paragraph("After"), b[2])
        // A line that starts with | but has no rule under it stays text.
        assertEquals(listOf(Block.Paragraph("| not a table")), blocks("| not a table"))
    }
}
