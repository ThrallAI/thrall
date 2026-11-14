package app.thrall.chat

/** What the chat list does when a row is added or the last one grows. */
enum class Scroll {
    /** The history is still coming in: nothing is drawn yet. */
    NONE,
    /** First drawing: straight to the latest message, no scrolling past the rest. */
    JUMP,
    /** Keep the latest message in view. */
    FOLLOW,
    /** They've scrolled up to read: stay put and offer a jump down. */
    MARK_NEW,
}

/** Opening at the latest message, and following new ones only while they're watching the end. */
object Follow {
    /** How far (px) the end of the last row may sit below the screen and still count as shown. */
    const val END_SLACK_PX = 48

    /**
     * Whether the END of the last row is on screen: [lastSeen] is the last row drawn, [bottom] where
     * it ends and [viewportEnd] where the screen ends. A long reply being written is one row, so
     * part of it showing isn't enough: reading higher up in it isn't watching the end.
     */
    fun endShown(lastSeen: Int, rowCount: Int, bottom: Int, viewportEnd: Int): Boolean =
        rowCount == 0 || (lastSeen >= rowCount - 1 && bottom <= viewportEnd + END_SLACK_PX)

    /**
     * [loaded]: the history replayed on opening is all in. [jumped]: the list has been put at the
     * latest message once. [following]: the end of the last row was in view when scrolling last
     * stopped, and they aren't touching the list now.
     * [mine]: the newest row is a message sent from here (always shown).
     */
    fun next(loaded: Boolean, jumped: Boolean, following: Boolean, mine: Boolean): Scroll = when {
        !loaded -> Scroll.NONE
        !jumped -> Scroll.JUMP
        following || mine -> Scroll.FOLLOW
        else -> Scroll.MARK_NEW
    }
}
