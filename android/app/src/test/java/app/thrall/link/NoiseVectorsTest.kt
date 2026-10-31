package app.thrall.link

import java.util.Base64
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phone's Noise must match the engine's (`snow`) byte for byte. The vectors come from
 * crates/thrall-server/tests/noise_vectors.rs, which checks the same file against `snow`.
 */
class NoiseVectorsTest {
    private fun hex(s: String): ByteArray = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    private val vectors: List<JsonObject> by lazy {
        val text = javaClass.classLoader!!.getResourceAsStream("noise_vectors.json")!!.readBytes().toString(Charsets.UTF_8)
        Json.parseToJsonElement(text).jsonObject["vectors"]!!.jsonArray.map { it.jsonObject }
    }

    private fun JsonObject.str(k: String) = this[k]!!.jsonPrimitive.content

    private fun handshake(v: JsonObject): Pair<Initiator, ByteArray> {
        val psk = v["psk"]?.jsonPrimitive?.content?.takeIf { it.isNotEmpty() }?.let(::hex)
        val phone = Initiator(
            localPrivate = hex(v.str("phone_static")),
            remotePublic = hex(v.str("engine_static_public")),
            prologue = hex(v.str("prologue")),
            psk = psk,
            fixedEphemeral = hex(v.str("phone_ephemeral")),
        )
        return phone to phone.writeMessage(hex(v.str("message1_payload")))
    }

    @Test
    fun bothPatternsMatchTheEngineByteForByte() {
        assertEquals(setOf(Noise.IK, Noise.IK_PSK1), vectors.map { it.str("pattern") }.toSet())
        for (v in vectors) {
            val name = v.str("pattern")
            assertArrayEquals("$name public key", hex(v.str("phone_static_public")), Noise.publicKey(hex(v.str("phone_static"))))
            val (phone, message1) = handshake(v)
            assertArrayEquals("$name message 1", hex(v.str("message1")), message1)
            val answer = phone.readMessage(hex(v.str("message2")))
            assertArrayEquals("$name answer payload", hex(v.str("message2_payload")), answer)
            val transport = phone.finish()
            assertArrayEquals("$name handshake hash", hex(v.str("handshake_hash")), transport.handshakeHash)

            // The phone's hello, in pieces, exactly as the engine expects.
            val pieces = Pieces.seal(transport.send, hex(v.str("phone_frame")))
            val expected = v["phone_pieces"]!!.jsonArray.map { hex(it.jsonPrimitive.content) }
            assertEquals(expected.size, pieces.size)
            expected.zip(pieces).forEach { (e, p) -> assertArrayEquals("$name phone piece", e, p) }

            // The engine's big frame comes back whole from its pieces.
            val joiner = Pieces.Joiner(transport.receive)
            val engine = v["engine_pieces"]!!.jsonArray.map { Base64.getDecoder().decode(it.jsonPrimitive.content) }
            assertTrue("$name: the engine frame needs several pieces", engine.size > 1)
            engine.dropLast(1).forEach { assertNull(joiner.add(it)) }
            val frame = joiner.add(engine.last())!!
            val len = v["engine_frame_len"]!!.jsonPrimitive.int
            val seed = v["engine_frame_seed"]!!.jsonPrimitive.int
            assertEquals(len, frame.size)
            frame.forEachIndexed { i, b -> if (b != ('a'.code + (i + seed) % 26).toByte()) throw AssertionError("$name frame byte $i") }
        }
    }

    @Test(expected = NoiseException::class)
    fun aWrongPairingSecretBreaksTheEnginesAnswer() {
        val v = vectors.first { it.str("pattern") == Noise.IK_PSK1 }
        val phone = Initiator(
            localPrivate = hex(v.str("phone_static")),
            remotePublic = hex(v.str("engine_static_public")),
            prologue = hex(v.str("prologue")),
            psk = ByteArray(32) { 9 },
            fixedEphemeral = hex(v.str("phone_ephemeral")),
        )
        phone.writeMessage(hex(v.str("message1_payload")))
        phone.readMessage(hex(v.str("message2")))
    }

    @Test(expected = NoiseException::class)
    fun aPieceOutOfOrderIsRefused() {
        val v = vectors.first { it.str("pattern") == Noise.IK }
        val (phone, _) = handshake(v)
        phone.readMessage(hex(v.str("message2")))
        val joiner = Pieces.Joiner(phone.finish().receive)
        val engine = v["engine_pieces"]!!.jsonArray.map { Base64.getDecoder().decode(it.jsonPrimitive.content) }
        joiner.add(engine[1])
    }
}
