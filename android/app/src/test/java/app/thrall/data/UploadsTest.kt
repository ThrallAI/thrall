package app.thrall.data

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.InputStream
import java.util.Base64

class UploadsTest {
    /** A fake engine: keeps every piece, gives the upload one id. */
    private class Engine {
        val pieces = mutableListOf<Pair<String?, ByteArray>>()
        val uploads = Uploads { upload, _, data ->
            pieces += upload to Base64.getDecoder().decode(data)
            "f_1"
        }
    }

    @Test
    fun aFileGoesInPiecesToOneUpload() = runBlocking {
        val engine = Engine()
        val bytes = ByteArray(Uploads.PIECE_BYTES * 2 + 10) { (it % 251).toByte() }
        val seen = mutableListOf<Long>()
        val id = engine.uploads.send("photo.jpg", ByteArrayInputStream(bytes)) { seen += it }
        assertEquals("f_1", id)
        assertEquals(3, engine.pieces.size)
        // The first piece starts the upload; the rest add to it.
        assertNull(engine.pieces[0].first)
        assertEquals(listOf("f_1", "f_1"), engine.pieces.drop(1).map { it.first })
        assertArrayEquals(bytes, engine.pieces.fold(ByteArray(0)) { all, p -> all + p.second })
        assertEquals(bytes.size.toLong(), seen.last())
    }

    @Test
    fun anExactPieceAndAnEmptyFileBothWork() = runBlocking {
        val exact = Engine()
        exact.uploads.send("a.bin", ByteArrayInputStream(ByteArray(Uploads.PIECE_BYTES)))
        // One full piece; the end is found without sending an empty one after it.
        assertEquals(listOf(Uploads.PIECE_BYTES), exact.pieces.map { it.second.size })
        val empty = Engine()
        assertEquals("f_1", empty.uploads.send("empty.txt", ByteArrayInputStream(ByteArray(0))))
        assertEquals(1, empty.pieces.size)
    }

    @Test
    fun aFileThatTrickles() = runBlocking {
        // Streams can hand over a few bytes at a time; pieces are still filled.
        val bytes = ByteArray(Uploads.PIECE_BYTES + 5) { 7 }
        val trickle = object : InputStream() {
            var at = 0
            override fun read(): Int = if (at < bytes.size) bytes[at++].toInt() else -1
            override fun read(b: ByteArray, off: Int, len: Int): Int {
                if (at >= bytes.size) return -1
                val n = minOf(len, 1000, bytes.size - at)
                System.arraycopy(bytes, at, b, off, n)
                at += n
                return n
            }
        }
        val engine = Engine()
        engine.uploads.send("slow.bin", trickle)
        assertEquals(listOf(Uploads.PIECE_BYTES, 5), engine.pieces.map { it.second.size })
    }

    @Test
    fun aFileOverTheLimitStopsBeforeSendingTooMuch() = runBlocking {
        val engine = Engine()
        val huge = object : InputStream() {
            override fun read(): Int = 1
        }
        try {
            engine.uploads.send("film.mp4", huge)
            fail("a file over the limit should be refused")
        } catch (e: RequestFailed) {
            assertEquals(Uploads.tooBig("film.mp4"), e.message)
        }
        assertTrue(engine.pieces.sumOf { it.second.size }.toLong() <= Uploads.MAX_BYTES)
    }
}
