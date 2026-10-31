package app.thrall.data

import app.thrall.protocol.AppRelease
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import java.util.Base64
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class UpdaterTest {
    private val apk = ByteArray(1_300) { (it % 251).toByte() }
    private val piece = 512

    private fun sha(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private fun release(sha: String = sha(apk), size: Long = apk.size.toLong()) = AppRelease(3, "0.3.0", size, sha)

    /** The engine's side: pieces of [bytes] from an offset. */
    private fun from(bytes: ByteArray): suspend (Long) -> Triple<Long, Long, String> = { offset ->
        val end = minOf(bytes.size, offset.toInt() + piece)
        Triple(offset, bytes.size.toLong(), Base64.getEncoder().encodeToString(bytes.copyOfRange(offset.toInt(), end)))
    }

    @Test
    fun a_release_arrives_whole_in_pieces() = runTest {
        val out = ByteArrayOutputStream()
        val seen = mutableListOf<Long>()
        Updater.fetch(release(), out, from(apk)) { seen += it }
        assertArrayEquals(apk, out.toByteArray())
        assertEquals(listOf(512L, 1024L, 1300L), seen)
    }

    @Test
    fun a_wrong_checksum_is_refused() = runTest {
        assertThrows(RequestFailed::class.java) {
            kotlinx.coroutines.runBlocking { Updater.fetch(release(sha = "00".repeat(32)), ByteArrayOutputStream(), from(apk)) }
        }
    }

    @Test
    fun a_file_that_changed_size_is_refused() = runTest {
        val bigger = apk + ByteArray(10)
        assertThrows(RequestFailed::class.java) {
            kotlinx.coroutines.runBlocking { Updater.fetch(release(), ByteArrayOutputStream(), from(bigger)) }
        }
    }
}
