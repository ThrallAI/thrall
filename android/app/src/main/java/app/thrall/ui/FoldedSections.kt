package app.thrall.ui

import android.content.Context

/** Which chat-list sections are folded, kept on the phone so opening a chat or the app keeps them. */
class FoldedSections(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    var keys: Set<String>
        get() = prefs.getStringSet(FOLDED, null)?.toSet().orEmpty()
        set(value) = prefs.edit().putStringSet(FOLDED, value).apply()

    private companion object {
        const val PREFS = "chat_list"
        const val FOLDED = "folded"
    }
}
