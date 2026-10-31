package app.thrall.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AttachmentsTest {
    private fun file(key: Long, ready: Boolean = true, problem: String? = null) =
        Attachment(key, "f$key.jpg", image = true, size = 100, file = if (ready) "f_$key" else null, problem = problem)

    @Test
    fun sendWaitsForEveryFile() {
        assertTrue(Attachments.canSend("hi", emptyList(), working = false))
        assertFalse(Attachments.canSend("  ", emptyList(), working = false))
        // Files alone are enough, once they're all on the computer.
        assertTrue(Attachments.canSend("", listOf(file(1), file(2)), working = false))
        assertFalse(Attachments.canSend("look", listOf(file(1), file(2, ready = false)), working = false))
        assertFalse(Attachments.canSend("look", listOf(file(1, ready = false, problem = "link dropped")), working = false))
    }

    @Test
    fun filesWaitWhileThrallWorksButWordsDont() {
        // The engine adds words to a running turn; files only start a new one.
        assertTrue(Attachments.canSend("also check the logs", emptyList(), working = true))
        assertFalse(Attachments.canSend("", listOf(file(1)), working = true))
    }

    @Test
    fun filesWithoutWordsGetAShortRequest() {
        assertEquals("what's this?", Attachments.text("  what's this? ", listOf(file(1))))
        assertEquals(Attachments.FILES_ONLY_ONE, Attachments.text("", listOf(file(1))))
        assertEquals(Attachments.FILES_ONLY_MANY, Attachments.text(" ", listOf(file(1), file(2))))
    }

    @Test
    fun progressAndRoom() {
        assertEquals(0.5f, Attachment(1, "a", false, size = 200, sent = 100).progress)
        assertEquals(0f, Attachment(1, "a", false, size = null, sent = 100).progress)
        assertTrue(Attachment(1, "a", false, size = 200).uploading)
        assertEquals(Attachments.MAX_FILES - 2, Attachments.room(listOf(file(1), file(2))))
        assertEquals(0, Attachments.room((1L..12L).map { file(it) }))
    }
}
