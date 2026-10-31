package app.thrall.link

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString

/**
 * One encrypted connection to the engine's phone link: the Noise handshake, then protocol frames
 * as text both ways. Callbacks arrive on OkHttp's thread.
 */
class Link(
    private val http: OkHttpClient,
    private val listener: Listener,
) {
    interface Listener {
        /** The handshake finished; [answer] is the engine's handshake payload. */
        fun onReady(answer: ByteArray)

        /** A whole protocol frame from the engine. */
        fun onFrame(text: String)

        /** The connection ended. [refused] is true when the engine closed it during the handshake. */
        fun onClosed(reason: String, refused: Boolean)
    }

    /** How a connection starts: pairing with a code, or a paired phone. */
    sealed class Hello {
        data class Pair(val codeId: ByteArray, val secret: ByteArray, val payload: ByteArray) : Hello()
        data object Connect : Hello()
    }

    private val lock = Any()
    private var socket: WebSocket? = null
    private var send: CipherState? = null
    private var joiner: Pieces.Joiner? = null
    private var handshake: Initiator? = null
    private var ended = false

    /** Open the link to [url] as the phone with [phonePrivate], talking to the engine [engineKey]. */
    fun open(url: String, phonePrivate: ByteArray, engineKey: ByteArray, hello: Hello) {
        val initiator = when (hello) {
            is Hello.Pair -> Initiator(phonePrivate, engineKey, PROLOGUE, psk = hello.secret)
            Hello.Connect -> Initiator(phonePrivate, engineKey, PROLOGUE)
        }
        val first = when (hello) {
            is Hello.Pair -> byteArrayOf(LINK_PAIR) + hello.codeId + initiator.writeMessage(hello.payload)
            Hello.Connect -> byteArrayOf(LINK_CONNECT) + initiator.writeMessage(ByteArray(0))
        }
        synchronized(lock) { handshake = initiator }
        http.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                synchronized(lock) { socket = webSocket }
                webSocket.send(first.toByteString())
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                try {
                    received(bytes.toByteArray())
                } catch (e: NoiseException) {
                    webSocket.close(CLOSE_NORMAL, null)
                    end("The connection's encryption failed: ${e.message}.", refused = false)
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(CLOSE_NORMAL, null)
                end("Thrall closed the connection.", refused = isHandshaking())
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                end("Thrall closed the connection.", refused = isHandshaking())
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val why = when (response?.code) {
                    null -> "Can't reach Thrall (${t.message ?: t.javaClass.simpleName})."
                    502, 503, 504 -> "The computer is reachable but Thrall isn't answering. Is Thrall running, with phones allowed in Settings → Phones?"
                    else -> "Thrall's address answered ${response.code}."
                }
                // A socket the engine drops mid-handshake is a refusal, not a network problem.
                end(why, refused = response == null && isHandshaking() && synchronized(lock) { socket != null })
            }
        })
    }

    private fun isHandshaking() = synchronized(lock) { handshake != null }

    private fun received(message: ByteArray) {
        val hs = synchronized(lock) { handshake }
        if (hs != null) {
            val answer = hs.readMessage(message)
            val transport = hs.finish()
            synchronized(lock) {
                send = transport.send
                joiner = Pieces.Joiner(transport.receive)
                handshake = null
            }
            listener.onReady(answer)
            return
        }
        val frame = synchronized(lock) { joiner }?.add(message) ?: return
        listener.onFrame(frame.toString(Charsets.UTF_8))
    }

    /** Send one protocol frame; false if the link isn't ready or has closed. */
    fun send(text: String): Boolean = synchronized(lock) {
        val ws = socket ?: return false
        val cipher = send ?: return false
        if (ended) return false
        // Sealing and queueing under one lock keeps the pieces in nonce order.
        Pieces.seal(cipher, text.toByteArray(Charsets.UTF_8)).all { ws.send(it.toByteString()) }
    }

    fun close() {
        val ws = synchronized(lock) { socket }
        ws?.close(CLOSE_NORMAL, null)
        end("Closed.", refused = false)
    }

    private fun end(reason: String, refused: Boolean) {
        val first = synchronized(lock) {
            val was = ended
            ended = true
            !was
        }
        if (first) listener.onClosed(reason, refused)
    }

    companion object {
        val PROLOGUE = "thrall-remote-v1".toByteArray(Charsets.US_ASCII)
        const val LINK_PAIR: Byte = 1
        const val LINK_CONNECT: Byte = 2
        private const val CLOSE_NORMAL = 1000
    }
}
