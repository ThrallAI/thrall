package app.thrall.voice

import android.content.Context

/** Voice settings kept on the phone: whether new replies are read aloud. */
class VoiceSettings(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    var readReplies: Boolean
        get() = prefs.getBoolean(READ_REPLIES, false)
        set(value) = prefs.edit().putBoolean(READ_REPLIES, value).apply()

    private companion object {
        const val PREFS = "voice"
        const val READ_REPLIES = "read_replies"
    }
}
