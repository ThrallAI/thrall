package app.thrall.link

import java.util.Base64

/**
 * What a pairing QR holds: `thrall://pair?a=<address>&k=<engine key>&i=<code id>&s=<secret>`, each
 * value base64url without padding.
 */
data class PairLink(val address: String, val engineKey: ByteArray, val codeId: ByteArray, val secret: ByteArray) {
    /** The phone link's WebSocket address: `https://host` becomes `wss://host/v1/remote`. */
    val socketUrl: String get() = socketUrl(address)

    override fun equals(other: Any?): Boolean =
        other is PairLink && address == other.address && engineKey.contentEquals(other.engineKey) &&
            codeId.contentEquals(other.codeId) && secret.contentEquals(other.secret)

    override fun hashCode(): Int = address.hashCode()

    companion object {
        const val PREFIX = "thrall://pair?"
        const val REMOTE_PATH = "/v1/remote"
        private const val ID_LEN = 16

        /** Read a pairing link; null if it isn't one. */
        fun parse(text: String): PairLink? {
            val query = text.trim().removePrefix(PREFIX).takeIf { it != text.trim() } ?: return null
            val fields = query.split('&').mapNotNull { kv ->
                val eq = kv.indexOf('=')
                if (eq <= 0) return@mapNotNull null
                val value = try {
                    Base64.getUrlDecoder().decode(kv.substring(eq + 1))
                } catch (e: IllegalArgumentException) {
                    return null
                }
                // Field names in either case (a keyboard may capitalise one); values are exact.
                kv.substring(0, eq).lowercase() to value
            }.toMap()
            val address = fields["a"]?.toString(Charsets.UTF_8) ?: return null
            val key = fields["k"]?.takeIf { it.size == Noise.KEY_LEN } ?: return null
            val id = fields["i"]?.takeIf { it.size == ID_LEN } ?: return null
            val secret = fields["s"]?.takeIf { it.size == Noise.KEY_LEN } ?: return null
            if (!address.startsWith("https://") && !address.startsWith("http://")) return null
            return PairLink(address, key, id, secret)
        }

        /** `https://host[/]` → `wss://host/v1/remote` (`http` → `ws`, for a computer nearby). */
        fun socketUrl(address: String): String {
            val base = address.trim().trimEnd('/')
            val ws = when {
                base.startsWith("https://") -> "wss://" + base.removePrefix("https://")
                base.startsWith("http://") -> "ws://" + base.removePrefix("http://")
                else -> base
            }
            return ws + REMOTE_PATH
        }
    }
}
