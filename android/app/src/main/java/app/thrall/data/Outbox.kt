package app.thrall.data

import android.content.Context
import app.thrall.protocol.Event
import app.thrall.protocol.Request
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json

/** A message typed while Thrall couldn't be reached, waiting to go. */
@Serializable
data class Queued(val id: String, val session: String, val text: String, val at: Long)

/**
 * Messages typed while the phone isn't connected: kept on the phone (they survive the app closing)
 * and sent, in order, as soon as the link is back. Text only; files need the link to upload.
 * A chat that's busy gets its next one when Thrall finishes there.
 */
class Outbox(context: Context, private val connection: Connection) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val _items = MutableStateFlow(load())
    val items: StateFlow<List<Queued>> = _items
    private val sending = Mutex()

    fun start(scope: CoroutineScope) {
        scope.launch { connection.state.collect { if (it is LinkState.Connected) flush() } }
        // A chat that was busy can take the next message once its turn is over.
        scope.launch {
            connection.events.collect { f -> if (f.event is Event.TurnCompleted && _items.value.any { it.session == f.session }) flush() }
        }
    }

    fun add(session: String, text: String) {
        val item = Queued("q-${System.currentTimeMillis()}-${_items.value.size}", session, text.trim(), System.currentTimeMillis())
        save(_items.value + item)
    }

    fun remove(id: String) = save(_items.value.filter { it.id != id })

    /** Send what's waiting, oldest first; a chat whose message can't go yet keeps the rest of its own. */
    suspend fun flush() = sending.withLock {
        val held = mutableSetOf<String>()
        for (item in _items.value.toList()) {
            if (item.session in held) continue
            try {
                connection.request(Request.TurnSend(item.session, item.text))
                remove(item.id)
            } catch (e: RequestFailed) {
                when (e.code) {
                    // The chat is gone: nothing to send it to.
                    GONE -> remove(item.id)
                    else -> held += item.session
                }
            } catch (e: Exception) {
                // The link dropped again: try everything next time.
                return@withLock
            }
        }
    }

    private fun load(): List<Queued> =
        prefs.getString(ITEMS, null)?.let { runCatching { json.decodeFromString(LIST, it) }.getOrNull() }.orEmpty()

    private fun save(items: List<Queued>) {
        _items.value = items
        prefs.edit().putString(ITEMS, json.encodeToString(LIST, items)).apply()
    }

    private companion object {
        const val PREFS = "outbox"
        const val ITEMS = "items"
        /** The engine's code for a session that isn't there. */
        const val GONE = "not_found"
        val LIST = ListSerializer(Queued.serializer())
        val json = Json { ignoreUnknownKeys = true }
    }
}
