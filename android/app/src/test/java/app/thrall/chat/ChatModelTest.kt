package app.thrall.chat

import app.thrall.protocol.Event
import app.thrall.protocol.ModeLimit
import app.thrall.protocol.Usage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatModelTest {
    @Test
    fun aTaskEndingInAutoEditsOffersAsk() {
        val m = ChatModel()
        m.opened("auto_edits", null)
        m.apply(Event.Status("working"), null)
        assertFalse(m.offerAsk)
        m.apply(Event.Status("idle"), null)
        assertTrue(m.offerAsk)
        m.apply(Event.SessionUpdated(mode = "ask"), 1)
        assertFalse(m.offerAsk)
        // In Ask there's nothing to offer.
        m.apply(Event.Status("working"), null)
        m.apply(Event.Status("idle"), null)
        assertFalse(m.offerAsk)
    }

    @Test
    fun fullAutoKeepsHowLongItLasts() {
        val m = ChatModel()
        m.apply(Event.SessionUpdated(mode = "full_auto", modeLimit = ModeLimit("task_end")), 1)
        assertEquals("task_end", m.modeLimit?.until)
        // It ends with the task by itself: no offer.
        m.apply(Event.Status("working"), null)
        m.apply(Event.Status("idle"), null)
        assertFalse(m.offerAsk)
        m.apply(Event.SessionUpdated(mode = "full_auto", modeLimit = ModeLimit("manual")), 2)
        m.apply(Event.Status("working"), null)
        m.apply(Event.Status("idle"), null)
        assertTrue(m.offerAsk)
        m.keepMode()
        assertFalse(m.offerAsk)
        // A title change leaves the mode alone; a mode change clears the limit.
        m.apply(Event.SessionUpdated(title = "New"), 3)
        assertEquals("manual", m.modeLimit?.until)
        m.apply(Event.SessionUpdated(mode = "ask"), 4)
        assertEquals(null, m.modeLimit)
        // What the session was opened with doesn't override what an event said.
        m.opened("full_auto", ModeLimit("manual"))
        assertEquals("ask", m.mode)
    }

    @Test
    fun aTurnStreamsThenSettles() {
        val m = ChatModel()
        m.apply(Event.TurnStarted("t1"), 1)
        m.apply(Event.MessageStarted("t1", "u1", "user"), 2)
        m.apply(Event.MessageCompleted("u1", "hello"), 3)
        m.apply(Event.MessageStarted("t1", "a1", "assistant"), 4)
        m.apply(Event.MessageDelta("a1", "Hi "), null)
        m.apply(Event.MessageDelta("a1", "there"), null)
        val streaming = m.rows.last() as ChatItem.Message
        assertEquals("Hi there", streaming.text)
        assertTrue(streaming.streaming)
        m.apply(Event.StepStarted("t1", "p1", "read_file", "Reading notes.txt"), 5)
        m.apply(Event.StepCompleted("p1", "ok", "12 lines"), 6)
        m.apply(Event.MessageCompleted("a1", "Hi there."), 7)
        m.apply(Event.TurnCompleted("t1", "done", Usage()), 8)
        val rows = m.rows
        assertEquals(3, rows.size)
        assertTrue((rows[0] as ChatItem.Message).fromUser)
        assertEquals("Hi there.", (rows[1] as ChatItem.Message).text)
        assertFalse((rows[1] as ChatItem.Message).streaming)
        assertEquals("12 lines", (rows[2] as ChatItem.Step).summary)
        assertEquals(8L, m.lastSeq)
    }

    @Test
    fun replayedEventsAreNotAppliedTwice() {
        val m = ChatModel()
        m.apply(Event.MessageStarted("t1", "a1", "assistant"), 1)
        m.apply(Event.MessageCompleted("a1", "once"), 2)
        m.apply(Event.MessageStarted("t1", "a1", "assistant"), 1)
        assertEquals("once", (m.rows.single() as ChatItem.Message).text)
    }

    @Test
    fun aRetractedTurnTakesLaterTurnsWithIt() {
        val m = ChatModel()
        for ((i, t) in listOf("t1", "t2", "t3").withIndex()) {
            m.apply(Event.TurnStarted(t), (i * 3 + 1).toLong())
            m.apply(Event.MessageStarted(t, "m-$t", "user"), (i * 3 + 2).toLong())
            m.apply(Event.TurnCompleted(t, "done"), (i * 3 + 3).toLong())
        }
        m.apply(Event.TurnRetracted("t2"), 10)
        assertEquals(listOf("m-t1"), m.rows.map { it.key })
    }

    @Test
    fun waitingCardsShowHowTheyWereSettled() {
        val m = ChatModel()
        m.apply(Event.TurnStarted("t1"), 1)
        m.apply(Event.PermissionRequest("r1", "t1", "p1", "shell", "Run `cargo test`", "medium", listOf("yes", "no", "always")), 2)
        assertEquals(listOf("yes", "no", "always"), (m.rows.single() as ChatItem.Waiting).choices)
        assertEquals(null, (m.rows.single() as ChatItem.Waiting).settled)
        m.apply(Event.PermissionResolved("r1", "yes", "desktop"), 3)
        val w = m.rows.single() as ChatItem.Waiting
        assertEquals("Allowed (desktop)", w.settled)
        m.apply(Event.Status("waiting_for_you", "x/y"), null)
        assertEquals("waiting_for_you", m.state)
        assertEquals("x/y", m.model)
    }

    @Test
    fun filesAStepMadeAreKept() {
        val m = ChatModel()
        m.apply(Event.StepStarted("t1", "p1", "make_image", "Drawing a cat"), 1)
        m.apply(Event.StepCompleted("p1", "ok", "1 picture", listOf(app.thrall.protocol.FileRef("f_1", "cat.png", "image", 1200))), 2)
        val step = m.rows.single() as ChatItem.Step
        assertEquals("cat.png", step.files.single().name)
    }

    @Test
    fun onlyRepliesFinishedLiveAreNew() {
        val m = ChatModel()
        // History, replayed: no deltas, so nothing to read aloud.
        m.apply(Event.TurnStarted("t1"), 1)
        m.apply(Event.MessageStarted("t1", "a1", "assistant"), 2)
        m.apply(Event.MessageCompleted("a1", "old reply"), 3)
        m.apply(Event.TurnCompleted("t1", "done"), 4)
        assertTrue(m.takeFinished().isEmpty())
        // Live: the turn's last reply, once the turn is done.
        m.apply(Event.TurnStarted("t2"), 5)
        m.apply(Event.MessageStarted("t2", "a2", "assistant"), 6)
        m.apply(Event.MessageDelta("a2", "Let me look"), null)
        m.apply(Event.MessageCompleted("a2", "Let me look."), 7)
        m.apply(Event.MessageStarted("t2", "a3", "assistant"), 8)
        m.apply(Event.MessageDelta("a3", "Found it"), null)
        m.apply(Event.MessageCompleted("a3", "Found it."), 9)
        assertTrue(m.takeFinished().isEmpty())
        m.apply(Event.TurnCompleted("t2", "done"), 10)
        assertEquals(listOf("Found it."), m.takeFinished().map { it.text })
        assertTrue(m.takeFinished().isEmpty())
        // Stopped part-way: not read.
        m.apply(Event.MessageStarted("t3", "a4", "assistant"), 11)
        m.apply(Event.MessageDelta("a4", "Half"), null)
        m.apply(Event.MessageCompleted("a4", "Half"), 12)
        m.apply(Event.TurnCompleted("t3", "interrupted"), 13)
        assertTrue(m.takeFinished().isEmpty())
    }

    @Test
    fun aMoveAndACommandsAnswerAreShown() {
        val m = ChatModel()
        m.apply(Event.SessionUpdated(workspace = "C:\\Work\\Board"), 1)
        assertEquals("C:\\Work\\Board", m.workspace)
        m.apply(Event.SessionUpdated(title = "Board"), 2)
        assertEquals("C:\\Work\\Board", m.workspace)
        m.commandResult("/status", "- **Model:** `x`")
        val row = m.rows.single() as ChatItem.Command
        assertEquals("/status", row.command)
    }
}
