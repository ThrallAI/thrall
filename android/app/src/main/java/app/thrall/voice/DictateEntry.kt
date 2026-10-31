package app.thrall.voice

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.TileService
import android.widget.RemoteViews
import app.thrall.MainActivity
import app.thrall.R

/** Opens the app in the last chat, listening. */
private fun dictateIntent(context: Context): Intent =
    Intent(context, MainActivity::class.java)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        .putExtra(MainActivity.EXTRA_DICTATE, true)

private fun dictatePending(context: Context): PendingIntent =
    PendingIntent.getActivity(context, DICTATE_REQUEST, dictateIntent(context), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)

/** The quick settings tile: tap it and talk to Thrall. */
class DictateTile : TileService() {
    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(dictatePending(this))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(dictateIntent(this))
        }
    }
}

/** The home-screen widget: one button that opens Thrall listening. */
class DictateWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val views = RemoteViews(context.packageName, R.layout.widget_dictate).apply {
            setOnClickPendingIntent(R.id.widget_dictate, dictatePending(context))
        }
        ids.forEach { manager.updateAppWidget(it, views) }
    }
}

private const val DICTATE_REQUEST = 7
