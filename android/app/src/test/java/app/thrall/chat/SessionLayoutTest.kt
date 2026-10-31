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
            s("b", "C:\\Code\\ark", group = "g1"),
            s("c", "C:\\Code\\ark", pinned = true, group = "g1"),
            s("d", null),
            s("e", "C:\\Code\\thrall-sandbox"),
            s("f", "C:\\Code\\ark", group = "gone"),
        )
        val groups = listOf(GroupInfo("g1", "ARK"), GroupInfo("g2", "Empty"))
        val out = SessionLayout.arrange(sessions, groups)
        assertEquals(listOf("Pinned", "ARK", "Empty", "thrall-sandbox", "No folder", "ark"), out.map { it.label })
        assertEquals(listOf("c"), out[0].sessions.map { it.session })
        assertEquals(listOf("b"), out[1].sessions.map { it.session })
        assertEquals(emptyList<String>(), out[2].sessions.map { it.session })
        assertEquals(listOf("a", "e"), out[3].sessions.map { it.session })
        // A chat in a group that no longer exists goes under its folder.
        assertEquals(listOf("f"), out[5].sessions.map { it.session })
        assertEquals("C:\\Code\\ark", out[5].folder)
    }
}
