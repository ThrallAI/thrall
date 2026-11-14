package app.thrall.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class FollowTest {
    @Test
    fun nothing_moves_while_the_history_comes_in() {
        assertEquals(Scroll.NONE, Follow.next(loaded = false, jumped = false, following = true, mine = false))
        assertEquals(Scroll.NONE, Follow.next(loaded = false, jumped = false, following = true, mine = true))
    }

    @Test
    fun opens_straight_at_the_latest_message() {
        assertEquals(Scroll.JUMP, Follow.next(loaded = true, jumped = false, following = true, mine = false))
        assertEquals(Scroll.JUMP, Follow.next(loaded = true, jumped = false, following = false, mine = false))
    }

    @Test
    fun follows_while_the_end_is_in_view() {
        assertEquals(Scroll.FOLLOW, Follow.next(loaded = true, jumped = true, following = true, mine = false))
    }

    @Test
    fun stays_put_when_scrolled_up() {
        assertEquals(Scroll.MARK_NEW, Follow.next(loaded = true, jumped = true, following = false, mine = false))
    }

    @Test
    fun own_message_always_shown() {
        assertEquals(Scroll.FOLLOW, Follow.next(loaded = true, jumped = true, following = false, mine = true))
    }

    @Test
    fun the_end_of_the_last_row_counts_not_just_part_of_it() {
        // A long reply (rows 0..4, row 4 drawn last) ending 2000 px down a 1500 px screen: reading
        // higher up in it isn't watching the end.
        assertEquals(false, Follow.endShown(lastSeen = 4, rowCount = 5, bottom = 2000, viewportEnd = 1500))
        // Its end on screen, or just below it, is.
        assertEquals(true, Follow.endShown(lastSeen = 4, rowCount = 5, bottom = 1400, viewportEnd = 1500))
        assertEquals(true, Follow.endShown(lastSeen = 4, rowCount = 5, bottom = 1500 + Follow.END_SLACK_PX, viewportEnd = 1500))
        // An earlier row drawn last: the newest isn't on screen at all.
        assertEquals(false, Follow.endShown(lastSeen = 3, rowCount = 5, bottom = 1400, viewportEnd = 1500))
        assertEquals(true, Follow.endShown(lastSeen = -1, rowCount = 0, bottom = 0, viewportEnd = 1500))
    }
}
