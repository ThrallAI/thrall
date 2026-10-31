package app.thrall.link

/**
 * Frames on the phone link go as encrypted pieces: each Noise message holds a marker byte (more
 * pieces follow, or this is the last) and up to [PIECE] bytes of the frame (docs/protocol.md §13a).
 */
object Pieces {
    const val MORE: Byte = 1
    const val LAST: Byte = 0

    /** Most frame bytes in one piece: the Noise limit less the tag and the marker. */
    const val PIECE = Noise.MAX_MESSAGE - Noise.TAG_LEN - 1

    /** Biggest frame accepted from the engine (twice the protocol's 1 MiB frame limit). */
    const val MAX_FRAME = 2 * 1024 * 1024

    /** A frame as encrypted pieces, in order. */
    fun seal(cipher: CipherState, frame: ByteArray): List<ByteArray> {
        val chunks = if (frame.isEmpty()) listOf(frame) else (frame.indices step PIECE).map {
            frame.copyOfRange(it, minOf(it + PIECE, frame.size))
        }
        return chunks.mapIndexed { i, chunk ->
            val marker = if (i == chunks.lastIndex) LAST else MORE
            cipher.encrypt(byteArrayOf(marker) + chunk)
        }
    }

    /** Joins decrypted pieces back into frames. */
    class Joiner(private val cipher: CipherState) {
        private val buffer = java.io.ByteArrayOutputStream()

        /** Decrypt one piece; the whole frame once its last piece is in, otherwise null. */
        fun add(piece: ByteArray): ByteArray? {
            val plain = cipher.decrypt(piece)
            if (plain.isEmpty()) throw NoiseException("a piece had no marker")
            if (buffer.size() + plain.size - 1 > MAX_FRAME) throw NoiseException("a frame was too big")
            buffer.write(plain, 1, plain.size - 1)
            if (plain[0] != LAST) return null
            return buffer.toByteArray().also { buffer.reset() }
        }
    }
}
