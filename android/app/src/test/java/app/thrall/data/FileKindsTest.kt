package app.thrall.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FileKindsTest {
    @Test
    fun filesAreShownTheRightWay() {
        assertEquals(FileKind.Markdown, FileKinds.kind("esp32-devboard/overview.md"))
        assertEquals(FileKind.Text, FileKinds.kind("src/main.rs"))
        assertEquals(FileKind.Image, FileKinds.kind("board.PNG"))
        assertEquals(FileKind.Other, FileKinds.kind("ESP32-WROOM32_dev_board.dxf"))
        assertEquals(FileKind.Other, FileKinds.kind("report.pdf"))
        assertEquals(FileKind.Other, FileKinds.kind("huge.log", size = 50L * 1024 * 1024))
        assertEquals("image/vnd.dxf", FileKinds.mime("a/b.DXF"))
    }

    @Test
    fun pathsInRepliesAreSpotted() {
        assertTrue(FileKinds.looksLikePath("esp32-devboard/overview.md"))
        assertTrue(FileKinds.looksLikePath("ESP32-WROOM32_dev_board.dxf"))
        assertTrue(FileKinds.looksLikePath("phone-test.txt"))
        assertFalse(FileKinds.looksLikePath("mcp_search"))
        assertFalse(FileKinds.looksLikePath("https://example.com/a.md"))
        assertFalse(FileKinds.looksLikePath("C:/Windows/win.ini"))
        assertFalse(FileKinds.looksLikePath("../secret.txt"))
        assertFalse(FileKinds.looksLikePath("rectang 0,0 62,34"))
        assertFalse(FileKinds.looksLikePath("v2.5"))
        assertFalse(FileKinds.looksLikePath(".gitignore"))
    }

    @Test
    fun wholePathsInsideTheFolderBecomeLinks() {
        val folder = "C:\\Users\\you\\Documents\\Thrall"
        assertEquals("phone-test/notes.md", FileKinds.pathIn("C:\\Users\\you\\Documents\\Thrall\\phone-test\\notes.md", folder))
        assertEquals("chart.png", FileKinds.pathIn("c:/users/you/documents/thrall/chart.png", folder))
        assertEquals("phone-test/data.csv", FileKinds.pathIn("phone-test\\data.csv", folder))
        assertEquals("notes.md", FileKinds.pathIn("./notes.md", null))
        // Outside the folder, or no folder to compare with: not a link.
        assertEquals(null, FileKinds.pathIn("C:\\Users\\you\\phone-test\\notes.md", folder))
        assertEquals(null, FileKinds.pathIn("C:\\Users\\you\\Documents\\Thrall2\\notes.md", folder))
        assertEquals(null, FileKinds.pathIn("C:\\Users\\you\\notes.md", null))
        assertEquals(null, FileKinds.pathIn("C:\\Users\\you\\Documents\\Thrall\\..\\secret.txt", folder))
    }

    @Test
    fun cachedNamesCantClimbOut() {
        assertEquals("a/b/c.md", Downloads.safePath("a/b/c.md"))
        // `..` parts are dropped, never followed.
        assertEquals("a/b/c.md", Downloads.safePath("a/../b/c.md"))
        assertEquals("b/c.md", Downloads.safePath("../b/c.md"))
        assertEquals("x_y.txt", Downloads.safe("x:y.txt"))
        assertEquals("file", Downloads.safe(".."))
    }
}
