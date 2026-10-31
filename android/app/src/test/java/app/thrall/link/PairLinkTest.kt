package app.thrall.link

import java.util.Base64
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairLinkTest {
    private fun b64(bytes: ByteArray) = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)

    private fun link(address: String = "https://home-pc.example.ts.net", key: Int = 32, id: Int = 16, secret: Int = 32) =
        "thrall://pair?a=${b64(address.toByteArray())}&k=${b64(ByteArray(key) { 1 })}&i=${b64(ByteArray(id) { 2 })}&s=${b64(ByteArray(secret) { 3 })}"

    @Test
    fun aPairingLinkIsRead() {
        val p = PairLink.parse(link())!!
        assertEquals("https://home-pc.example.ts.net", p.address)
        assertArrayEquals(ByteArray(32) { 1 }, p.engineKey)
        assertArrayEquals(ByteArray(16) { 2 }, p.codeId)
        assertArrayEquals(ByteArray(32) { 3 }, p.secret)
        assertEquals("wss://home-pc.example.ts.net/v1/remote", p.socketUrl)
        // A keyboard capitalising a field name doesn't break it.
        assertEquals(p, PairLink.parse(link().replace("&i=", "&I=")))
    }

    @Test
    fun anythingElseIsNot() {
        assertNull(PairLink.parse("https://example.com"))
        assertNull(PairLink.parse(link(key = 31)))
        assertNull(PairLink.parse(link(id = 8)))
        assertNull(PairLink.parse(link(address = "ftp://x")))
        assertNull(PairLink.parse("thrall://pair?a=%%%"))
    }

    @Test
    fun addressesBecomeSocketUrls() {
        assertEquals("wss://pc.ts.net/v1/remote", PairLink.socketUrl("https://pc.ts.net/"))
        assertEquals("ws://10.0.2.2:47919/v1/remote", PairLink.socketUrl("http://10.0.2.2:47919"))
    }
}
