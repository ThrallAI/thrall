package app.thrall.chat

import app.thrall.protocol.Diff
import app.thrall.protocol.Event
import app.thrall.protocol.FileRef
import app.thrall.protocol.ModeLimit
import app.thrall.protocol.QuestionItem
import app.thrall.protocol.SavedFile

/** One row of a conversation. */
sealed interface ChatItem {
    val key: String
    val turn: String?

    data class Message(
        override val key: String,
        override val turn: String?,
        val fromUser: Boolean,
        val text: String,
        val streaming: Boolean,
        val files: List<FileRef> = emptyList(),
    ) : ChatItem

    /** A tool call: collapsed to its title and result. */
    data class Step(
        override val key: String,
        override val turn: String?,
        val tool: String,
        val title: String,
        /** `running`, or how it ended: `ok` · `error` · `denied` · `cancelled`. */
        val status: String,
        val summary: String = "",
        val output: String = "",
        /** Files the step made (pictures, documents). */
        val files: List<FileRef> = emptyList(),
        /** The file in the session's folder it made or changed, if it did. */
        val changed: Diff? = null,
        /** Files it wrote on the computer, wherever they are (fetched by their place in this list). */
        val saved: List<SavedFile> = emptyList(),
    ) : ChatItem

    /** Thrall is waiting on a permission or a plan decision. */
    data class Waiting(
        override val key: String,
        override val turn: String?,
        val what: String,
        val plan: Boolean,
        /** How it was settled, once it was. */
        val settled: String? = null,
        /** For a permission: the answers on offer (`yes`, `no`, `always`). */
        val choices: List<String> = emptyList(),
        /** For a question card: Thrall's questions, each with its recommended answer. */
        val questions: List<QuestionItem> = emptyList(),
        /** For a question card: when Thrall goes ahead with its picks, Unix ms. */
        val expiresAt: Long = 0,
    ) : ChatItem

    data class Note(override val key: String, override val turn: String?, val text: String, val warning: Boolean) : ChatItem

    /** A slash command typed on this phone and what the engine answered (not part of the log). */
    data class Command(override val key: String, val command: String, val markdown: String) : ChatItem {
        override val turn: String? get() = null
    }
}

/**
 * Builds a conversation from its events (replayed from the log, then live). Pure: no Android, so
 * it's tested on its own.
 */
class ChatModel {
    private val items = LinkedHashMap<String, ChatItem>()
    private val turns = ArrayList<String>()
    private var currentTurn: String? = null
    private var notes = 0

    // Replies seen streaming here (deltas are never replayed, so these are live, not history).
    private val heard = HashSet<String>()
    private var lastReply: ChatItem.Message? = null
    private val finished = ArrayList<ChatItem.Message>()

    /** Last stored event applied, for reconnecting with `after_seq`. */
    var lastSeq: Long = 0
        private set

    /** `idle` · `working` · `waiting_for_you`. */
    var state: String = "idle"
        private set
    var model: String = ""
        private set
    var title: String? = null
        private set

    /** The folder the session works in, once the engine says it moved (`/folder`). */
    var workspace: String? = null
        private set

    /** The permission mode, once the engine says it changed. */
    var mode: String? = null
        private set

    /** How long Full auto lasts, while it's on. */
    var modeLimit: ModeLimit? = null
        private set

    /**
     * A task just ended with the chat still in a mode that doesn't ask: offer to go back to Ask.
     * (Full auto that ends with the task switches itself back, so it needs no offer.)
     */
    var offerAsk: Boolean = false
        private set

    /** The mode the session had when it was opened, until an event says otherwise. */
    fun opened(mode: String, limit: ModeLimit?) {
        if (this.mode != null) return
        this.mode = mode
        modeLimit = if (mode == FULL_AUTO) limit else null
    }

    /** The offer to go back to Ask was turned down. */
    fun keepMode() {
        offerAsk = false
    }

    val rows: List<ChatItem> get() = items.values.toList()

    fun apply(event: Event, seq: Long?) {
        if (seq != null) {
            // Replayed after a reconnect: already applied.
            if (seq <= lastSeq) return
            lastSeq = seq
        }
        when (event) {
            is Event.TurnStarted -> {
                currentTurn = event.turn
                if (event.turn !in turns) turns += event.turn
            }
            is Event.TurnCompleted -> {
                // Anything still streaming is as finished as it'll get.
                items.replaceAll { _, it -> if (it is ChatItem.Message && it.streaming) it.copy(streaming = false) else it }
                items.replaceAll { _, it -> if (it is ChatItem.Step && it.status == RUNNING) it.copy(status = "cancelled") else it }
                if (currentTurn == event.turn) currentTurn = null
                // The turn's last reply, if it streamed while watching: new, for reading aloud.
                lastReply?.let { if (event.status == "done") finished += it }
                lastReply = null
            }
            is Event.TurnRetracted -> retract(event.turn)
            is Event.MessageStarted -> {
                if (event.turn !in turns) turns += event.turn
                items[event.message] = ChatItem.Message(event.message, event.turn, event.role == "user", "", streaming = true)
            }
            is Event.MessageDelta -> {
                heard += event.message
                message(event.message) { it.copy(text = it.text + event.text) }
            }
            is Event.MessageCompleted -> {
                val known = items[event.message] as? ChatItem.Message
                items[event.message] = (known ?: ChatItem.Message(event.message, currentTurn, false, "", false))
                    .copy(text = event.text, streaming = false, files = event.files)
                val done = items[event.message] as ChatItem.Message
                if (event.message in heard && !done.fromUser && done.text.isNotBlank()) lastReply = done
            }
            is Event.StepStarted -> items[event.step] = ChatItem.Step(event.step, event.turn, event.tool, event.title, RUNNING)
            is Event.StepOutput -> step(event.step) { it.copy(output = (it.output + event.text).takeLast(OUTPUT_KEEP)) }
            is Event.StepCompleted -> step(event.step) {
                it.copy(status = event.status, summary = event.summary, files = event.files, changed = event.diff, saved = event.saved)
            }
            is Event.PermissionRequest ->
                items[event.request] = ChatItem.Waiting(event.request, event.turn, event.summary, plan = false, choices = event.choices)
            is Event.PermissionResolved -> waiting(event.request) { it.copy(settled = "${answer(event.choice)} (${event.answeredBy})") }
            is Event.PlanProposed -> items[event.plan] = ChatItem.Waiting(event.plan, currentTurn, event.markdown, plan = true)
            is Event.PlanResolved -> waiting(event.plan) {
                it.copy(settled = "${if (event.approved) "Approved" else "Not approved"} (${event.answeredBy})")
            }
            is Event.QuestionAsk -> items[event.question] = ChatItem.Waiting(
                event.question, currentTurn, event.why, plan = false, questions = event.items, expiresAt = event.expiresAt,
            )
            is Event.QuestionResolved -> waiting(event.question) {
                val answers = event.answers.joinToString(" · ")
                it.copy(
                    settled = if (event.answeredBy == QUESTION_TIMEOUT) "No answer in time, so Thrall went with its picks: $answers"
                    else "Answered (${event.answeredBy}): $answers",
                )
            }
            is Event.Status -> {
                val ended = event.state == IDLE && state != IDLE
                if (ended && seq == null && keepsGoing()) offerAsk = true
                if (event.state != IDLE) offerAsk = false
                state = event.state
                if (event.model.isNotEmpty()) model = event.model
            }
            is Event.SessionUpdated -> {
                event.title?.let { title = it }
                event.workspace?.let { workspace = it }
                event.mode?.let {
                    mode = it
                    modeLimit = if (it == FULL_AUTO) event.modeLimit else null
                    offerAsk = false
                }
            }
            is Event.Notice -> note(event.text, event.level == "warning")
            is Event.Error -> note(event.message, warning = true)
            is Event.ReasoningDelta, is Event.AgentCompleted, Event.Unknown -> Unit
        }
    }

    /** Replies finished live since the last call (each turn's last one), oldest first. */
    fun takeFinished(): List<ChatItem.Message> = finished.toList().also { finished.clear() }

    /** Show a command's answer at the end of the conversation. */
    fun commandResult(command: String, markdown: String) {
        val key = "command-${notes++}"
        items[key] = ChatItem.Command(key, command, markdown)
    }

    private fun note(text: String, warning: Boolean) {
        val key = "note-${notes++}"
        items[key] = ChatItem.Note(key, currentTurn, text, warning)
    }

    private fun retract(turn: String) {
        val from = turns.indexOf(turn)
        val gone = if (from >= 0) turns.subList(from, turns.size).toSet() else setOf(turn)
        items.values.removeAll { it.turn in gone }
        if (from >= 0) turns.subList(from, turns.size).clear()
        if (currentTurn in gone) currentTurn = null
    }

    private inline fun message(key: String, f: (ChatItem.Message) -> ChatItem.Message) {
        (items[key] as? ChatItem.Message)?.let { items[key] = f(it) }
    }

    private inline fun step(key: String, f: (ChatItem.Step) -> ChatItem.Step) {
        (items[key] as? ChatItem.Step)?.let { items[key] = f(it) }
    }

    private inline fun waiting(key: String, f: (ChatItem.Waiting) -> ChatItem.Waiting) {
        (items[key] as? ChatItem.Waiting)?.let { items[key] = f(it) }
    }

    private fun answer(choice: String) = when (choice) {
        "yes" -> "Allowed"
        "always" -> "Always allowed"
        else -> "Not allowed"
    }

    /** In a mode that goes ahead without asking, and stays in it after the task. */
    private fun keepsGoing() =
        mode == AUTO_EDITS || (mode == FULL_AUTO && modeLimit?.until != UNTIL_TASK_END)

    companion object {
        const val RUNNING = "running"
        /** Live output kept for a running step (the last part matters). */
        const val OUTPUT_KEEP = 4_000
        const val IDLE = "idle"
        const val AUTO_EDITS = "auto_edits"
        const val FULL_AUTO = "full_auto"
        /** Full auto ends when the task does (`ModeLimit.until`). */
        const val UNTIL_TASK_END = "task_end"
        const val UNTIL_TIME = "time"
        const val UNTIL_MANUAL = "manual"
        /** `answered_by` on a question card nobody answered in time. */
        const val QUESTION_TIMEOUT = "timeout"
    }
}
