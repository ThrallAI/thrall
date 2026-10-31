package app.thrall.link

import java.security.SecureRandom
import org.bouncycastle.crypto.digests.Blake2sDigest
import org.bouncycastle.crypto.macs.HMac
import org.bouncycastle.crypto.modes.ChaCha20Poly1305
import org.bouncycastle.crypto.params.AEADParameters
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.math.ec.rfc7748.X25519

/**
 * The Noise protocol, as far as the phone link needs it: the phone is always the initiator of
 * `Noise_IK_25519_ChaChaPoly_BLAKE2s` (a paired phone connecting) or
 * `Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s` (pairing with a one-time secret). Written to the Noise
 * specification (revision 34) on BouncyCastle's primitives, and checked byte for byte against the
 * engine's implementation by the shared test vectors.
 */
object Noise {
    const val IK = "Noise_IK_25519_ChaChaPoly_BLAKE2s"
    const val IK_PSK1 = "Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s"
    const val KEY_LEN = 32
    const val HASH_LEN = 32
    const val TAG_LEN = 16
    const val MAX_MESSAGE = 65_535

    private val random = SecureRandom()

    /** A fresh X25519 key pair (private, public). */
    fun keyPair(): Pair<ByteArray, ByteArray> {
        val private = ByteArray(KEY_LEN)
        X25519.generatePrivateKey(random, private)
        return private to publicKey(private)
    }

    /** The public key for an X25519 private key. */
    fun publicKey(private: ByteArray): ByteArray {
        val public = ByteArray(KEY_LEN)
        X25519.generatePublicKey(private, 0, public, 0)
        return public
    }

    internal fun dh(private: ByteArray, public: ByteArray): ByteArray {
        val out = ByteArray(KEY_LEN)
        if (!X25519.calculateAgreement(private, 0, public, 0, out, 0)) {
            throw NoiseException("the other side's key is not usable")
        }
        return out
    }

    internal fun hash(vararg parts: ByteArray): ByteArray {
        val d = Blake2sDigest(HASH_LEN * 8)
        for (p in parts) d.update(p, 0, p.size)
        return ByteArray(HASH_LEN).also { d.doFinal(it, 0) }
    }

    private fun hmac(key: ByteArray, vararg parts: ByteArray): ByteArray {
        val mac = HMac(Blake2sDigest(HASH_LEN * 8))
        mac.init(KeyParameter(key))
        for (p in parts) mac.update(p, 0, p.size)
        return ByteArray(HASH_LEN).also { mac.doFinal(it, 0) }
    }

    /** HKDF with HMAC-BLAKE2s: two or three outputs. */
    internal fun hkdf(chainingKey: ByteArray, input: ByteArray, outputs: Int): List<ByteArray> {
        val temp = hmac(chainingKey, input)
        val out1 = hmac(temp, byteArrayOf(1))
        val out2 = hmac(temp, out1, byteArrayOf(2))
        if (outputs == 2) return listOf(out1, out2)
        val out3 = hmac(temp, out2, byteArrayOf(3))
        return listOf(out1, out2, out3)
    }

    private fun nonce(n: Long): ByteArray {
        val b = ByteArray(12)
        for (i in 0 until 8) b[4 + i] = (n ushr (8 * i)).toByte()
        return b
    }

    internal fun seal(key: ByteArray, n: Long, ad: ByteArray, plain: ByteArray): ByteArray {
        val c = ChaCha20Poly1305()
        c.init(true, AEADParameters(KeyParameter(key), TAG_LEN * 8, nonce(n), ad))
        val out = ByteArray(c.getOutputSize(plain.size))
        val len = c.processBytes(plain, 0, plain.size, out, 0)
        c.doFinal(out, len)
        return out
    }

    internal fun open(key: ByteArray, n: Long, ad: ByteArray, sealed: ByteArray): ByteArray {
        val c = ChaCha20Poly1305()
        c.init(false, AEADParameters(KeyParameter(key), TAG_LEN * 8, nonce(n), ad))
        val out = ByteArray(c.getOutputSize(sealed.size))
        try {
            val len = c.processBytes(sealed, 0, sealed.size, out, 0)
            c.doFinal(out, len)
        } catch (e: Exception) {
            throw NoiseException("a message didn't decrypt")
        }
        return out
    }
}

/** Something about the link's encryption failed; the connection can't go on. */
class NoiseException(message: String) : Exception(message)

/** One direction of an encrypted link: a key and the count of messages so far. */
class CipherState internal constructor(private val key: ByteArray) {
    private var n = 0L

    /** Encrypt one transport message. */
    fun encrypt(plain: ByteArray): ByteArray {
        check(n != -1L) { "nonce exhausted" }
        return Noise.seal(key, n++, ByteArray(0), plain)
    }

    /** Decrypt one transport message; they must arrive in order. */
    fun decrypt(sealed: ByteArray): ByteArray = Noise.open(key, n, ByteArray(0), sealed).also { n++ }
}

/** Both directions of a finished handshake. */
class Transport internal constructor(val send: CipherState, val receive: CipherState, val handshakeHash: ByteArray)

/**
 * The phone's side of a handshake. Write the first message, read the engine's answer, then
 * [finish] for the transport keys.
 *
 * @param psk the pairing secret (IKpsk1), or null for IK.
 * @param fixedEphemeral only for tests: a known ephemeral key instead of a random one.
 */
class Initiator(
    private val localPrivate: ByteArray,
    private val remotePublic: ByteArray,
    prologue: ByteArray,
    private val psk: ByteArray? = null,
    private val fixedEphemeral: ByteArray? = null,
) {
    private val name = if (psk == null) Noise.IK else Noise.IK_PSK1
    private var h: ByteArray
    private var ck: ByteArray
    private var k: ByteArray? = null
    private var n = 0L
    private var ephemeral: ByteArray? = null
    private var step = 0

    init {
        require(psk == null || psk.size == Noise.KEY_LEN) { "a pairing secret is 32 bytes" }
        val bytes = name.toByteArray(Charsets.US_ASCII)
        h = if (bytes.size <= Noise.HASH_LEN) bytes.copyOf(Noise.HASH_LEN) else Noise.hash(bytes)
        ck = h.copyOf()
        mixHash(prologue)
        // IK's pre-message: the responder's static key is known.
        mixHash(remotePublic)
    }

    private fun mixHash(data: ByteArray) {
        h = Noise.hash(h, data)
    }

    private fun mixKey(input: ByteArray) {
        val (newCk, key) = Noise.hkdf(ck, input, 2)
        ck = newCk
        k = key
        n = 0
    }

    private fun mixKeyAndHash(input: ByteArray) {
        val (newCk, tempH, key) = Noise.hkdf(ck, input, 3)
        ck = newCk
        mixHash(tempH)
        k = key
        n = 0
    }

    private fun encryptAndHash(plain: ByteArray): ByteArray {
        val key = k ?: return plain.also { mixHash(it) }
        val sealed = Noise.seal(key, n++, h, plain)
        mixHash(sealed)
        return sealed
    }

    private fun decryptAndHash(data: ByteArray): ByteArray {
        val key = k ?: return data.also { mixHash(it) }
        val plain = Noise.open(key, n++, h, data)
        mixHash(data)
        return plain
    }

    /** `-> e, es, s, ss` (then `psk` when pairing), carrying [payload]. */
    fun writeMessage(payload: ByteArray): ByteArray {
        check(step == 0) { "the first message was already written" }
        step = 1
        val ePrivate = fixedEphemeral ?: Noise.keyPair().first
        ephemeral = ePrivate
        val ePublic = Noise.publicKey(ePrivate)
        mixHash(ePublic)
        if (psk != null) mixKey(ePublic)
        mixKey(Noise.dh(ePrivate, remotePublic))
        val s = encryptAndHash(Noise.publicKey(localPrivate))
        mixKey(Noise.dh(localPrivate, remotePublic))
        if (psk != null) mixKeyAndHash(psk)
        val body = encryptAndHash(payload)
        return ePublic + s + body
    }

    /** `<- e, ee, se`: the engine's answer; returns its payload. */
    fun readMessage(message: ByteArray): ByteArray {
        check(step == 1) { "write the first message first" }
        if (message.size < Noise.KEY_LEN + Noise.TAG_LEN) throw NoiseException("the answer is too short")
        step = 2
        val re = message.copyOfRange(0, Noise.KEY_LEN)
        mixHash(re)
        if (psk != null) mixKey(re)
        val e = ephemeral ?: throw NoiseException("no ephemeral key")
        mixKey(Noise.dh(e, re))
        mixKey(Noise.dh(localPrivate, re))
        return decryptAndHash(message.copyOfRange(Noise.KEY_LEN, message.size))
    }

    /** The transport keys: the phone sends with the first, receives with the second. */
    fun finish(): Transport {
        check(step == 2) { "the handshake isn't finished" }
        val (send, receive) = Noise.hkdf(ck, ByteArray(0), 2)
        return Transport(CipherState(send), CipherState(receive), h.copyOf())
    }
}
