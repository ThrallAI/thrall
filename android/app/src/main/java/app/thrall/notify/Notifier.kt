package app.thrall.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.thrall.MainActivity
import app.thrall.R
import app.thrall.data.Connection
import app.thrall.data.LinkState
import app.thrall.lock.AppLock
import app.thrall.protocol.Event
import app.thrall.protocol.Request
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Tells the person, while the app is out of sight, when Thrall needs them (a permission card or a
 * plan) or finished something. Permission cards can be answered from the notification itself,
 * unless the app lock is on: then the notification only opens the app. Nothing shows on the lock
 * screen beyond "Thrall".
 */
class Notifier(
    private val context: Context,
    private val connection: Connection,
    private val lock: AppLock,
    private val visible: () -> Boolean,
) {
    private val manager = NotificationManagerCompat.from(context)

    fun start(scope: CoroutineScope) {
        channels()
        scope.launch {
            // Every session's events (live text left out), for each new link.
            connection.state.collect { if (it is LinkState.Connected) runCatching { connection.request(Request.SessionWatchAll) } }
        }
        scope.launch {
            connection.events.collect { f ->
                when (val e = f.event) {
                    is Event.PermissionRequest -> if (!visible()) needsYou(f.session, e)
                    is Event.PlanProposed -> if (!visible()) plan(f.session, e.plan)
                    is Event.PermissionResolved -> manager.cancel(TAG, e.request.hashCode())
                    is Event.PlanResolved -> manager.cancel(TAG, e.plan.hashCode())
                    is Event.TurnCompleted -> if (!visible() && e.status != CANCELLED) finished(f.session, e.turn, e.status)
                    else -> Unit
                }
            }
        }
    }

    private fun needsYou(session: String, e: Event.PermissionRequest) {
        val id = e.request.hashCode()
        val n = base(CHANNEL_NEEDS_YOU, session, id)
            .setContentTitle("Thrall needs you")
            .setContentText(e.summary)
            .setStyle(NotificationCompat.BigTextStyle().bigText(e.summary))
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
        if (!lock.enabled) {
            for (choice in e.choices) {
                val label = when (choice) {
                    "yes" -> "Allow"
                    "no" -> "Deny"
                    "always" -> "Always allow"
                    else -> continue
                }
                n.addAction(0, label, AnswerReceiver.intent(context, e.request, choice, id))
            }
        }
        post(id, n)
    }

    private fun plan(session: String, plan: String) {
        val id = plan.hashCode()
        post(id, base(CHANNEL_NEEDS_YOU, session, id).setContentTitle("Thrall has a plan for you").setContentText("Open it to approve or change it."))
    }

    private fun finished(session: String, turn: String, status: String) {
        val id = turn.hashCode()
        val text = if (status == DONE) "Thrall finished." else "Thrall stopped before finishing."
        post(id, base(CHANNEL_FINISHED, session, id).setContentTitle(text).setContentText("Tap to see the chat."))
    }

    private fun base(channel: String, session: String, id: Int): NotificationCompat.Builder =
        NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_launcher_monochrome)
            .setAutoCancel(true)
            .setContentIntent(open(context, session, id))
            // On the lock screen: only that Thrall has something, never what.
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(
                NotificationCompat.Builder(context, channel)
                    .setSmallIcon(R.drawable.ic_launcher_monochrome)
                    .setContentTitle("Thrall")
                    .build(),
            )

    private fun post(id: Int, n: NotificationCompat.Builder) {
        if (!allowed(context)) return
        try {
            manager.notify(TAG, id, n.build())
        } catch (e: SecurityException) {
            // The person took the permission away in the meantime.
        }
    }

    /** A note after answering from a notification didn't work. */
    fun problem(id: Int, text: String) {
        post(id, base(CHANNEL_NEEDS_YOU, "", id).setContentTitle("Couldn't answer").setContentText(text))
    }

    fun cancel(id: Int) = manager.cancel(TAG, id)

    private fun channels() {
        val m = context.getSystemService(NotificationManager::class.java) ?: return
        m.createNotificationChannel(NotificationChannel(CHANNEL_NEEDS_YOU, "Needs you", NotificationManager.IMPORTANCE_HIGH).apply {
            description = "Thrall is waiting for your answer or a plan decision."
        })
        m.createNotificationChannel(NotificationChannel(CHANNEL_FINISHED, "Finished", NotificationManager.IMPORTANCE_DEFAULT).apply {
            description = "Thrall finished something while the app was closed."
        })
        m.createNotificationChannel(NotificationChannel(CHANNEL_CONNECTION, "Staying connected", NotificationManager.IMPORTANCE_MIN).apply {
            description = "The quiet notification Android needs while Thrall stays connected."
            setShowBadge(false)
        })
    }

    companion object {
        const val CHANNEL_NEEDS_YOU = "needs_you"
        const val CHANNEL_FINISHED = "finished"
        const val CHANNEL_CONNECTION = "connection"
        /** Which chat a notification opens. */
        const val EXTRA_SESSION = "app.thrall.session"
        private const val TAG = "thrall"
        private const val DONE = "done"
        private const val CANCELLED = "cancelled"

        /** Whether Android lets the app post notifications. */
        fun allowed(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

        /** Open the app at [session] ("" for the list). */
        fun open(context: Context, session: String, request: Int): PendingIntent {
            val intent = Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra(EXTRA_SESSION, session)
            return PendingIntent.getActivity(context, request, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }
    }
}
