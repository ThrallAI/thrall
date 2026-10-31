package app.thrall.notify

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import app.thrall.R
import app.thrall.ThrallApp
import app.thrall.data.LinkState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/** Whether the app keeps its link to Thrall while it's closed (on unless switched off). */
class StaySettings(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    var enabled: Boolean
        get() = prefs.getBoolean(ENABLED, true)
        set(value) = prefs.edit().putBoolean(ENABLED, value).apply()

    private companion object {
        const val PREFS = "stay"
        const val ENABLED = "enabled"
    }
}

/**
 * Keeps the app running, and so its link to Thrall open, while it's out of sight, so "needs you"
 * and "finished" notifications arrive. Android requires a lasting notification for this; it's
 * kept as quiet as Android allows. No relay or Google service is involved: the phone talks to the
 * computer directly, as it does when open.
 */
class StayConnected : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var watching: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val app = application as ThrallApp
        show(app.connection.state.value)
        if (watching == null) {
            watching = scope.launch {
                app.connection.state.collect { state ->
                    if (state is LinkState.NotPaired || state is LinkState.Refused) {
                        stopSelf()
                    } else {
                        show(state)
                    }
                }
            }
        }
        app.connection.start()
        return START_STICKY
    }

    private fun show(state: LinkState) {
        val text = when (state) {
            is LinkState.Connected -> "Connected to your computer"
            else -> "Reconnecting to your computer…"
        }
        val n = NotificationCompat.Builder(this, Notifier.CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_launcher_monochrome)
            .setContentTitle("Thrall")
            .setContentText(text)
            .setOngoing(true)
            .setSilent(true)
            .setContentIntent(Notifier.open(this, "", ONGOING_ID))
            .build()
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING
        } else {
            0
        }
        ServiceCompat.startForeground(this, ONGOING_ID, n, type)
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val ONGOING_ID = 1

        /** Start or stop it to match the setting (call while the app is in front). */
        fun sync(context: Context, on: Boolean) {
            val intent = Intent(context, StayConnected::class.java)
            if (on) {
                runCatching { context.startForegroundService(intent) }
            } else {
                context.stopService(intent)
            }
        }
    }
}
