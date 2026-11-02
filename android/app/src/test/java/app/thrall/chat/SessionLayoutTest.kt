package app.thrall.chat

import app.thrall.protocol.GroupInfo
import app.thrall.protocol.SessionInfo
import org.junit.Assert.assertEquals
import org.junit.Test

class SessionLayoutTest {
    private fun s(id: String, folder: String? = null, group: String? = null, pinned: Boolean = false) =
        SessionInfo(id, id, workspace = folder, group = group, pinned = pinned)

    @Test
    fun pinned_then_groups_then_folders_newest_first() {
        val sessions = listOf(
            s("a", "C:\\Code\\thrall-sandbox"),
            s("b", "C:\\Code\\garden", group = "g1"),
            s("c", "C:\\Code\\garden", pinned = true, group = "g1"),
            s("d", null),
            s("e", "C:\\Code\\thrall-sandbox"),
            s("f", "C:\\Code\\garden", group = "gone"),
        )
        val groups = listOf(GroupInfo("g1", "Garden"), GroupInfo("g2", "Empty"))
        val out = SessionLayout.arrange(sessions, groups)
        assertEquals(listOf("Pinned", "Garden", "Empty", "thrall-sandbox", "No folder", "garden"), out.map { it.label })
        assertEquals(listOf("c"), out[0].sessions.map { it.session })
        assertEquals(listOf("b"), out[1].sessions.map { it.session })
        assertEquals(emptyList<String>(), out[2].sessions.map { it.session })
        assertEquals(listOf("a", "e"), out[3].sessions.map { it.session })
        // A chat in a group that no longer exists goes under its folder.
        assertEquals(listOf("f"), out[5].sessions.map { it.session })
        assertEquals("C:\\Code\\garden", out[5].folder)
    }
}
