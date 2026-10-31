package app.thrall.data

import app.thrall.BuildConfig
import app.thrall.link.Link
import app.thrall.link.Noise
import app.thrall.link.PairLink
import app.thrall.protocol.ClientInfo
import app.thrall.protocol.Incoming
import app.thrall.protocol.Protocol
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import app.thrall.protocol.UserInfo
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.selects.onTimeout
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient

/** Where the link to the engine stands, for the status strip. */
sealed interface LinkState {
    /** No engine paired yet. */
    data object NotPaired : LinkState

    data object Connecting : LinkState

    /** Connected, as [user]: whose phone this is, as the engine says. */
    data class Connected(val engineVersion: String, val model: String, val user: UserInfo) : LinkState

    /** Not connected; trying again at [retryAt] (or sooner, when the network comes back). */
    data class Offline(val reason: String, val retryAt: Long) : LinkState

    /** The engine turned this phone away (removed on the computer): pair again. */
    data class Refused(val reason: String) : LinkState
}

/** The request failed: not connected, no answer in time, or the engine said no. */
class RequestFailed(message: String, val code: String = "") : Exception(message)

/**
 * The app's one connection to the engine: it connects when there's a pairing, reconnects with
 * growing waits when the link drops, matches replies to requests, and passes session events on.
 */
class Connection(private val store: PairingStore) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val http = OkHttpClient.Builder()
        // WebSocket pings keep the engine's dead-connection check happy and notice a dead network.
        .pingInterval(PING_SECONDS, TimeUnit.SECONDS)
        .connectTimeout(CONNECT_SECONDS, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)
        .build()

    private val _state = MutableStateFlow<LinkState>(if (store.load() == null) LinkState.NotPaired else LinkState.Connecting)
    val state: StateFlow<LinkState> = _state

    /** Whose phone this is, as the engine last said (kept while offline; cleared on unpairing). */
    private val _me = MutableStateFlow<UserInfo?>(null)
    val me: StateFlow<UserInfo?> = _me

    private val _events = MutableSharedFlow<Incoming.EventFrame>(extraBufferCapacity = EVENT_BUFFER)
    val events: SharedFlow<Incoming.EventFrame> = _events

    private val pending = ConcurrentHashMap<String, CompletableDeferred<Reply>>()
    private val ids = AtomicLong()
    private val wake = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var link: Link? = null
    private var loop: Job? = null

    /** Connect (and keep reconnecting) if the phone is paired. */
    @Synchronized
    fun start() {
        if (loop?.isActive == true) return
        loop = scope.launch { run() }
    }

    /** Try again now instead of waiting (the network came back, or the person asked). */
    fun retryNow() {
        if (loop?.isActive != true) start() else wake.trySend(Unit)
    }

    /**
     * Make sure the link really works (the app came back on screen, or the phone changed network):
     * a link that looks connected can have died quietly while the phone slept, and would only be
     * noticed minutes later. No answer to a ping soon means reconnect now.
     */
    fun checkLink() {
        if (_state.value !is LinkState.Connected) {
            retryNow()
            return
        }
        scope.launch {
            val alive = runCatching { request(Request.Ping, PROBE_TIMEOUT_MS) }.isSuccess
            if (!alive) link?.close()
        }
    }

    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    private suspend fun run() {
        var wait = FIRST_WAIT_MS
        while (true) {
            val pairing = store.load() ?: run {
                _state.value = LinkState.NotPaired
                return
            }
            _state.value = LinkState.Connecting
            val end = connectOnce(pairing)
            if (end.refused) {
                _state.value = LinkState.Refused(
                    "Thrall didn't recognise this phone. It may have been removed on the computer: pair it again.",
                )
                return
            }
            if (end.connected) wait = FIRST_WAIT_MS
            _state.value = LinkState.Offline(end.reason, System.currentTimeMillis() + wait)
            select {
                wake.onReceive {}
                onTimeout(wait) {}
            }
            wait = minOf(wait * 2, MAX_WAIT_MS)
        }
    }

    private data class End(val reason: String, val refused: Boolean, val connected: Boolean)

    /** One connection, from opening to closing. */
    private suspend fun connectOnce(pairing: Pairing): End {
        val ended = CompletableDeferred<End>()
        var connected = false
        val l = Link(http, object : Link.Listener {
            override fun onReady(answer: ByteArray) {
                scope.launch {
                    try {
                        val welcome = request(hello()) as? Reply.Welcome
                            ?: throw RequestFailed("Thrall didn't say hello back.")
                        connected = true
                        _me.value = welcome.user
                        _state.value = LinkState.Connected(welcome.engine.version, welcome.engine.model, welcome.user)
                    } catch (e: Exception) {
                        link?.close()
                    }
                }
            }

            override fun onFrame(text: String) = received(text)

            override fun onClosed(reason: String, refused: Boolean) {
                link = null
                failPending(reason)
                ended.complete(End(reason, refused, connected))
            }
        })
        link = l
        l.open(PairLink.socketUrl(pairing.address), pairing.phonePrivate, pairing.engineKey, Link.Hello.Connect)
        return ended.await()
    }

    private fun hello() = Request.Hello(
        protocols = listOf(Protocol.VERSION),
        client = ClientInfo(kind = "android", name = "Thrall for Android", version = BuildConfig.VERSION_NAME),
    )

    private fun received(text: String) {
        when (val frame = Protocol.parse(text) ?: return) {
            is Incoming.ReplyFrame -> pending.remove(frame.re)?.complete(frame.reply)
            is Incoming.EventFrame -> _events.tryEmit(frame)
        }
    }

    private fun failPending(reason: String) {
        val waiting = pending.values.toList()
        pending.clear()
        waiting.forEach { it.completeExceptionally(RequestFailed(reason)) }
    }

    /** Send a request and wait for its reply. An `error` reply comes back as [RequestFailed]. */
    suspend fun request(request: Request, timeoutMs: Long = REQUEST_TIMEOUT_MS): Reply {
        val l = link ?: throw RequestFailed("Not connected to Thrall.")
        val id = "a-${ids.incrementAndGet()}"
        val answer = CompletableDeferred<Reply>()
        pending[id] = answer
        if (!l.send(Protocol.frame(id, request))) {
            pending.remove(id)
            throw RequestFailed("Not connected to Thrall.")
        }
        val reply = try {
            withTimeout(timeoutMs) { answer.await() }
        } catch (e: TimeoutCancellationException) {
            pending.remove(id)
            throw RequestFailed("Thrall didn't answer in time.")
        }
        if (reply is Reply.Error) throw RequestFailed(reply.message, reply.code)
        return reply
    }

    /** Send a request without waiting for it (for when the screen asking is going away). */
    fun sendLater(request: Request) {
        scope.launch { runCatching { request(request) } }
    }

    /**
     * Pair with the engine in a scanned (or pasted) link: a fresh key for this phone, proved with
     * the one-time secret. On success the pairing is saved and the app connects.
     */
    suspend fun pair(link: PairLink, phoneName: String): Result<Unit> {
        loop?.cancel()
        this.link?.close()
        val (private, _) = Noise.keyPair()
        val done = CompletableDeferred<Result<String>>()
        val payload = """{"name":${kotlinx.serialization.json.JsonPrimitive(phoneName)}}""".toByteArray()
        lateinit var l: Link
        l = Link(http, object : Link.Listener {
            override fun onReady(answer: ByteArray) {
                val id = runCatching {
                    Protocol.json.parseToJsonElement(answer.toString(Charsets.UTF_8)).jsonObject["phone"]!!.jsonPrimitive.content
                }.getOrNull()
                done.complete(if (id != null) Result.success(id) else Result.failure(RequestFailed("Thrall's answer didn't make sense.")))
                l.close()
            }

            override fun onFrame(text: String) = Unit

            override fun onClosed(reason: String, refused: Boolean) {
                done.complete(
                    Result.failure(
                        RequestFailed(
                            if (refused) {
                                "The code didn't work. It may have run out or been used already: make a new one in Settings → Phones on the computer."
                            } else {
                                reason
                            },
                        ),
                    ),
                )
            }
        })
        l.open(link.socketUrl, private, link.engineKey, Link.Hello.Pair(link.codeId, link.secret, payload))
        val result = try {
            withTimeout(PAIR_TIMEOUT_MS) { done.await() }
        } catch (e: TimeoutCancellationException) {
            l.close()
            Result.failure(RequestFailed("Thrall didn't answer. Check the computer is on and phones are allowed."))
        }
        return result.map { phoneId ->
            store.save(Pairing(link.address, link.engineKey, phoneId, private))
            start()
        }.also { if (it.isFailure && store.load() != null) start() }
    }

    /** Forget the pairing on this phone (the computer still lists it until removed there). */
    fun unpair() {
        loop?.cancel()
        link?.close()
        store.forget()
        _me.value = null
        _state.value = LinkState.NotPaired
    }

    private companion object {
        const val PING_SECONDS = 20L
        const val CONNECT_SECONDS = 15L
        const val EVENT_BUFFER = 1024
        const val FIRST_WAIT_MS = 1_000L
        const val MAX_WAIT_MS = 30_000L
        const val REQUEST_TIMEOUT_MS = 30_000L
        /** How long [checkLink] waits for the engine's answer before reconnecting. */
        const val PROBE_TIMEOUT_MS = 5_000L
        const val PAIR_TIMEOUT_MS = 20_000L
    }
}
