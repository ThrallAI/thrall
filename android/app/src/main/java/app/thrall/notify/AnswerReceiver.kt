package app.thrall.notify

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.thrall.ThrallApp
import app.thrall.data.RequestFailed
import app.thrall.protocol.Request
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** Allow / Deny tapped on a "Thrall needs you" notification. */
class AnswerReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = context.applicationContext as ThrallApp
        val request = intent.getStringExtra(EXTRA_REQUEST) ?: return
        val choice = intent.getStringExtra(EXTRA_CHOICE) ?: return
        val id = intent.getIntExtra(EXTRA_ID, 0)
        // The app lock may have been switched on since the notification was posted.
        if (app.lock.enabled) {
            app.notifier.problem(id, "App lock is on: open Thrall to answer.")
            return
        }
        val done = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                app.connection.request(Request.PermissionAnswer(request, choice))
                app.notifier.cancel(id)
            } catch (e: RequestFailed) {
                if (e.code == ALREADY_ANSWERED) app.notifier.cancel(id) else app.notifier.problem(id, e.message ?: "Open Thrall to answer.")
            } catch (e: Exception) {
                app.notifier.problem(id, "Not connected to Thrall: open it to answer.")
            } finally {
                done.finish()
            }
        }
    }

    companion object {
        private const val EXTRA_REQUEST = "request"
        private const val EXTRA_CHOICE = "choice"
        private const val EXTRA_ID = "id"
        private const val ALREADY_ANSWERED = "already_answered"

        fun intent(context: Context, request: String, choice: String, id: Int): PendingIntent {
            val i = Intent(context, AnswerReceiver::class.java)
                .putExtra(EXTRA_REQUEST, request)
                .putExtra(EXTRA_CHOICE, choice)
                .putExtra(EXTRA_ID, id)
            // One PendingIntent per answer of each card.
            return PendingIntent.getBroadcast(context, (request + choice).hashCode(), i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }
    }
}
