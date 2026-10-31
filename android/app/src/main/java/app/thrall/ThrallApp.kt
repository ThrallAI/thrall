package app.thrall

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import app.thrall.data.Connection
import app.thrall.data.Downloads
import app.thrall.data.LinkState
import app.thrall.data.Outbox
import app.thrall.data.PairingStore
import app.thrall.data.PhoneFiles
import app.thrall.data.Updater
import app.thrall.data.Uploads
import app.thrall.lock.AppLock
import app.thrall.notify.Notifier
import app.thrall.notify.StaySettings
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch

/** Holds the one connection to the engine for the whole app. */
class ThrallApp : Application() {
    lateinit var store: PairingStore
        private set
    lateinit var connection: Connection
        private set
    lateinit var downloads: Downloads
        private set
    lateinit var uploads: Uploads
        private set
    lateinit var phoneFiles: PhoneFiles
        private set
    lateinit var updater: Updater
        private set
    lateinit var lock: AppLock
        private set
    lateinit var notifier: Notifier
        private set
    lateinit var stay: StaySettings
        private set

    /** The app is on screen (notifications wait until it isn't). */
    @Volatile var visible = false

    lateinit var outbox: Outbox
        private set

    /** A chat a notification asked to open; the screen takes it and sets it back to null. */
    val openChat = MutableStateFlow<String?>(null)

    /** Files or text shared into Thrall (from another app, or a file sent on from a chat), waiting for a chat. */
    val incoming = MutableStateFlow<Shared?>(null)

    /** Shared things given to a chat; that chat's screen takes them and sets this back to null. */
    val handoff = MutableStateFlow<Pair<String, Shared>?>(null)

    /** Opened to dictate (the widget or quick tile): the last chat opens listening. */
    val dictate = MutableStateFlow(false)

    private val prefs by lazy { getSharedPreferences(PREFS, MODE_PRIVATE) }

    /** The chat last opened on this phone, for the widget and quick tile. */
    var lastChat: String?
        get() = prefs.getString(LAST_CHAT, null)
        set(value) = prefs.edit().putString(LAST_CHAT, value).apply()

    override fun onCreate() {
        super.onCreate()
        store = PairingStore(this)
        connection = Connection(store)
        downloads = Downloads(cacheDir, connection)
        uploads = Uploads(connection)
        phoneFiles = PhoneFiles(this)
        updater = Updater(this, connection)
        lock = AppLock(this)
        stay = StaySettings(this)
        notifier = Notifier(this, connection, lock) { visible }
        outbox = Outbox(this, connection)
        connection.start()
        val scope = appScope
        notifier.start(scope)
        outbox.start(scope)
        // Each time the link comes up, see whether the computer offers a newer app.
        scope.launch {
            connection.state.collect { if (it is LinkState.Connected) runCatching { updater.check() } }
        }
        // When the phone gets a network back (Wi-Fi to mobile data, out of a tunnel), reconnect now;
        // a link that was open on the old network has usually died with it.
        getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = connection.checkLink()
            },
        )
    }

    /** Work that lasts as long as the app does. */
    private val appScope = MainScope()

    /**
     * See whether the computer offers a newer app: the app came back on screen. (A phone that
     * stays connected never sees the link come up again, so checking only then missed releases.)
     */
    fun checkForUpdate() {
        if (connection.state.value is LinkState.Connected) appScope.launch { runCatching { updater.check() } }
    }

    private companion object {
        const val PREFS = "app"
        const val LAST_CHAT = "last_chat"
    }
}

/** Files (links the app may read) and text to put in a chat's message bar. */
data class Shared(val uris: List<android.net.Uri>, val text: String)
