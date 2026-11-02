package app.thrall.ui

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.thrall.ThrallApp
import app.thrall.chat.Attachment
import app.thrall.chat.Attachments
import app.thrall.chat.ChatItem
import app.thrall.chat.ChatModel
import app.thrall.chat.SlashCommands
import app.thrall.protocol.CommandItem
import app.thrall.data.Connection
import app.thrall.data.LinkState
import app.thrall.data.Outbox
import app.thrall.data.Queued
import app.thrall.data.PhoneFiles
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import app.thrall.data.RequestFailed
import app.thrall.data.Uploads
import app.thrall.protocol.Event
import app.thrall.protocol.GroupInfo
import app.thrall.protocol.ModeLimit
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import app.thrall.protocol.SessionInfo
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** The session list and Home. */
class SessionsModel(app: Application) : AndroidViewModel(app) {
    private val connection = (app as ThrallApp).connection

    data class Shown(
        val home: SessionInfo? = null,
        val sessions: List<SessionInfo> = emptyList(),
        val loading: Boolean = true,
        val problem: String? = null,
        /** Showing archived chats instead of the usual list. */
        val archived: Boolean = false,
        /** The person's groups, in order. */
        val groups: List<GroupInfo> = emptyList(),
    )

    private val _shown = MutableStateFlow(Shown())
    val shown: StateFlow<Shown> = _shown

    private var pending: Job? = null

    init {
        viewModelScope.launch {
            connection.state.collect {
                if (it is LinkState.Connected) {
                    // Every session's status, so "Needs you" shows on the list as it happens.
                    runCatching { connection.request(Request.SessionWatchAll) }
                    refresh()
                }
            }
        }
        viewModelScope.launch {
            connection.events.collect { f ->
                when (val e = f.event) {
                    // A session starting, finishing or waiting: update its row straight away.
                    is Event.Status -> setState(f.session, e.state)
                    // Finished work or a new title: the list is read again (once for a burst).
                    is Event.TurnCompleted, is Event.SessionUpdated, is Event.AgentCompleted -> refreshSoon()
                    else -> Unit
                }
            }
        }
    }

    private fun setState(session: String, state: String) {
        val s = _shown.value
        _shown.value = s.copy(
            home = s.home?.let { if (it.session == session) it.copy(state = state) else it },
            sessions = s.sessions.map { if (it.session == session) it.copy(state = state) else it },
        )
    }

    private fun refreshSoon() {
        pending?.cancel()
        pending = viewModelScope.launch {
            delay(REFRESH_SETTLE_MS)
            refresh()
        }
    }

    fun refresh() {
        viewModelScope.launch {
            _shown.value = _shown.value.copy(loading = true)
            val archived = _shown.value.archived
            try {
                val home = (connection.request(Request.HomeGet) as? Reply.HomeShown)?.page
                val list = (connection.request(Request.SessionList(limit = LIST_SIZE, archived = archived.takeIf { it })) as? Reply.SessionListed)
                    ?.items.orEmpty()
                // Pinned chats first, the rest newest first (as the engine sends them).
                val sorted = list.filter { !it.home }.sortedByDescending { it.pinned }
                val groups = runCatching { (connection.request(Request.GroupList) as? Reply.GroupListed)?.items }.getOrNull().orEmpty()
                _shown.value = Shown(home, sorted, loading = false, archived = archived, groups = groups)
            } catch (e: Exception) {
                _shown.value = _shown.value.copy(loading = false, problem = e.message)
            }
        }
    }

    /** Switch between the usual list and archived chats. */
    fun showArchived(archived: Boolean) {
        _shown.value = _shown.value.copy(archived = archived, sessions = emptyList())
        refresh()
    }

    fun rename(session: String, title: String) = change { Request.SessionRename(session, title.trim()) }
    fun pin(session: String, pinned: Boolean) = change { Request.SessionPin(session, pinned) }
    fun archive(session: String, archived: Boolean) = change { Request.SessionArchive(session, archived) }
    fun delete(session: String) = change { Request.SessionDelete(session) }
    fun move(session: String, group: String?) = change { Request.SessionMove(session, group) }

    /** A new group, with [session] moved into it if given. */
    fun newGroup(name: String, session: String?) {
        viewModelScope.launch {
            try {
                val made = connection.request(Request.GroupCreate(name.trim())) as? Reply.GroupListed
                val group = made?.items?.lastOrNull { it.name == name.trim() }
                if (session != null && group != null) connection.request(Request.SessionMove(session, group.group))
            } catch (e: Exception) {
                _shown.value = _shown.value.copy(problem = e.message ?: "Couldn't make the group.")
            }
            refresh()
        }
    }

    fun renameGroup(group: String, name: String) = change { Request.GroupRename(group, name.trim()) }
    fun deleteGroup(group: String) = change { Request.GroupDelete(group) }

    /** Move a group one place up ([by] -1) or down (+1). */
    fun shiftGroup(group: String, by: Int) {
        val order = _shown.value.groups.map { it.group }.toMutableList()
        val at = order.indexOf(group)
        val to = at + by
        if (at < 0 || to !in order.indices) return
        order.add(to, order.removeAt(at))
        change { Request.GroupReorder(order) }
    }

    fun dismissProblem() {
        _shown.value = _shown.value.copy(problem = null)
    }

    /** Send a change to a chat, then read the list again. */
    private fun change(request: () -> Request) {
        viewModelScope.launch {
            try {
                connection.request(request())
                _shown.value = _shown.value.copy(problem = null)
            } catch (e: Exception) {
                _shown.value = _shown.value.copy(problem = e.message ?: "That didn't work.")
            }
            refresh()
        }
    }

    /** Start a new conversation working in [folder]; [opened] gets it, [failed] why it couldn't start. */
    fun newChat(folder: String?, opened: (SessionInfo) -> Unit, failed: (String) -> Unit) {
        viewModelScope.launch {
            try {
                val s = (connection.request(Request.SessionCreate(workspace = folder)) as? Reply.SessionOpened)?.info
                    ?: return@launch
                connection.request(Request.SessionDetach(s.session))
                opened(s)
            } catch (e: Exception) {
                failed(e.message ?: "Couldn't start the chat.")
            }
        }
    }

    private companion object {
        const val LIST_SIZE = 50
        /** Wait after a session changes before reading the list again (a burst becomes one read). */
        const val REFRESH_SETTLE_MS = 600L
    }
}

/**
 * One open conversation: its rows, built from its events, and sending to it. Lives as long as the
 * chat screen (in [scope]); [close] detaches, and the session keeps running on the computer.
 */
class ChatModelView(
    private val connection: Connection,
    private val session: String,
    private val scope: CoroutineScope,
    private val uploads: Uploads,
    private val phoneFiles: PhoneFiles,
    private val outbox: Outbox,
) {
    /** Messages typed for this chat while offline, waiting to go. */
    val queued: StateFlow<List<Queued>> = outbox.items.map { all -> all.filter { it.session == session } }
        .stateIn(scope, SharingStarted.Eagerly, emptyList())

    private val _draft = MutableStateFlow<String?>(null)
    /** Text to put in the message bar (shared from another app); the bar takes it. */
    val draft: StateFlow<String?> = _draft

    fun setDraft(text: String) {
        if (text.isNotBlank()) _draft.value = text
    }

    fun draftTaken() {
        _draft.value = null
    }

    /** Take back a message that hasn't gone yet. */
    fun unqueue(id: String) = outbox.remove(id)

    private val _editing = MutableStateFlow(false)
    /** The next message replaces the last one (`turn.edit`) instead of following it. */
    val editing: StateFlow<Boolean> = _editing

    /** Change the last message: its text goes back in the box, and sending replaces it. */
    fun startEdit(text: String) {
        _editing.value = true
        _draft.value = text
    }

    fun cancelEdit() {
        _editing.value = false
    }

    /** Send the last message again (Thrall's answer to it is taken back first). */
    fun retry() {
        scope.launch {
            try {
                connection.request(Request.TurnRetry(session))
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    /** What rewinding to before [turn] would do; null with a problem shown if it can't. */
    suspend fun previewRewind(turn: String): Reply.CheckpointShown? = try {
        connection.request(Request.CheckpointPreview(session, turn)) as? Reply.CheckpointShown
    } catch (e: Exception) {
        publish(problem = e.message)
        null
    }

    /** Go back to before [turn]: files put back and the conversation taken back; its message returns to the box. */
    fun rewind(turn: String, files: Boolean) {
        scope.launch {
            try {
                val done = connection.request(Request.TurnRewind(session, turn, files = files, conversation = true)) as? Reply.TurnRewound
                done?.text?.let { setDraft(it) }
                if (!done?.failed.isNullOrEmpty()) {
                    publish(problem = "These files couldn't be put back: ${done?.failed?.joinToString()}")
                }
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }
    private val model = ChatModel()

    private val _attachments = MutableStateFlow<List<Attachment>>(emptyList())
    /** Files picked for the next message, uploading as soon as they're picked. */
    val attachments: StateFlow<List<Attachment>> = _attachments
    private val picked = mutableMapOf<Long, Uri>()
    private val uploading = mutableMapOf<Long, Job>()
    private var nextKey = 0L

    data class Shown(
        val rows: List<ChatItem> = emptyList(),
        val title: String = "",
        val state: String = "idle",
        val model: String = "",
        val problem: String? = null,
        /** The folder the session works in; null when none was picked. */
        val workspace: String? = null,
        /** The permission mode: `ask`, `auto_edits`, `plan` or `full_auto`. */
        val mode: String = "ask",
        /** How long Full auto lasts, while it's on. */
        val modeLimit: ModeLimit? = null,
        /** A task just ended in a mode that doesn't ask: offer to go back to Ask. */
        val offerAsk: Boolean = false,
    )

    private val _shown = MutableStateFlow(Shown())
    val shown: StateFlow<Shown> = _shown
    private var title = ""
    private var workspace: String? = null
    private var mode = "ask"

    private val _replies = MutableSharedFlow<ChatItem.Message>(extraBufferCapacity = REPLIES_BUFFER)
    /** Thrall's replies as they finish while the chat is open (for reading aloud). */
    val replies: SharedFlow<ChatItem.Message> = _replies

    private val _commands = MutableStateFlow<List<CommandItem>>(emptyList())
    /** The slash commands, for the list shown while typing `/`. */
    val commands: StateFlow<List<CommandItem>> = _commands

    init {
        // Listen before opening, so nothing the engine replays is missed.
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            connection.events.collect { f ->
                if (f.session != session) return@collect
                val finished = synchronized(model) {
                    model.apply(f.event, f.seq)
                    model.takeFinished()
                }
                publish()
                finished.forEach { _replies.tryEmit(it) }
            }
        }
        scope.launch {
            // (Re)attach whenever the link comes up, asking only for what was missed; files that
            // failed to upload because the link dropped start again.
            connection.state.collect {
                if (it is LinkState.Connected) {
                    open()
                    _attachments.value.filter { a -> a.problem != null && !a.tooBig }.forEach { a -> upload(a.key) }
                }
            }
        }
    }

    /** Add files picked on the phone; each starts uploading at once. */
    fun attach(uris: List<Uri>) {
        val room = Attachments.room(_attachments.value)
        if (uris.size > room) publish(problem = "A message can carry up to ${Attachments.MAX_FILES} files.")
        for (uri in uris.take(room)) {
            val info = runCatching { phoneFiles.info(uri) }.getOrNull() ?: continue
            val key = nextKey++
            picked[key] = uri
            val tooBig = (info.size ?: 0) > Uploads.MAX_BYTES
            _attachments.update {
                it + Attachment(key, info.name, info.image, info.size, problem = if (tooBig) Uploads.tooBig(info.name) else null, tooBig = tooBig)
            }
            if (!tooBig) upload(key)
        }
    }

    /** The phone's link to a picked file (for its preview). */
    fun uriOf(key: Long): Uri? = picked[key]

    /** Send a picked file again (after it failed). */
    fun retry(key: Long) {
        if (_attachments.value.any { it.key == key && !it.tooBig }) upload(key)
    }

    /** Take a picked file off the message. */
    fun remove(key: Long) {
        uploading.remove(key)?.cancel()
        picked.remove(key)
        _attachments.update { list -> list.filter { it.key != key } }
    }

    private fun change(key: Long, edit: (Attachment) -> Attachment) =
        _attachments.update { list -> list.map { if (it.key == key) edit(it) else it } }

    private fun upload(key: Long) {
        val uri = picked[key] ?: return
        val name = _attachments.value.firstOrNull { it.key == key }?.name ?: return
        uploading.remove(key)?.cancel()
        uploading[key] = scope.launch {
            change(key) { it.copy(sent = 0, file = null, problem = null) }
            try {
                val id = withContext(Dispatchers.IO) {
                    phoneFiles.open(uri).use { input -> uploads.send(name, input) { n -> change(key) { a -> a.copy(sent = n) } } }
                }
                change(key) { it.copy(file = id, sent = it.size ?: it.sent) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val tooBig = e.message == Uploads.tooBig(name)
                change(key) { it.copy(problem = e.message ?: "Couldn't send $name.", tooBig = tooBig) }
            }
        }
    }

    private suspend fun open() {
        try {
            val after = synchronized(model) { model.lastSeq }.takeIf { it > 0 }
            val opened = connection.request(Request.SessionOpen(session, after)) as? Reply.SessionOpened
            if (opened != null) {
                title = opened.title
                workspace = opened.workspace
                mode = opened.mode
                synchronized(model) { model.opened(opened.mode, opened.modeLimit) }
            }
            publish(problem = null)
            if (_commands.value.isEmpty()) loadCommands()
        } catch (e: Exception) {
            publish(problem = e.message)
        }
    }

    private fun publish(problem: String? = _shown.value.problem) {
        _shown.value = synchronized(model) {
            Shown(
                model.rows, model.title ?: title, model.state, model.model, problem, model.workspace ?: workspace,
                model.mode ?: mode, model.modeLimit, model.offerAsk,
            )
        }
    }

    private suspend fun loadCommands() {
        runCatching { connection.request(Request.CommandList) as? Reply.CommandListed }
            .getOrNull()?.let { _commands.value = it.items }
    }

    /**
     * A message (with the picked files), or a slash command: the engine's run here, skills go to
     * Thrall as messages.
     */
    fun send(text: String) {
        val files = _attachments.value
        if (files.isNotEmpty()) {
            if (!Attachments.canSend(text, files, working = _shown.value.state != IDLE)) return
            scope.launch {
                try {
                    connection.request(Request.TurnSend(session, Attachments.text(text, files), files.mapNotNull { it.file }))
                    files.forEach { remove(it.key) }
                } catch (e: Exception) {
                    publish(problem = e.message)
                }
            }
            return
        }
        if (text.isBlank()) return
        // Not connected: keep it on the phone and send it when Thrall's back (commands need Thrall now).
        if (connection.state.value !is LinkState.Connected) {
            if (SlashCommands.parse(text) != null || _editing.value) {
                publish(problem = "That needs the connection. Try again when Thrall's back.")
            } else {
                outbox.add(session, text)
            }
            return
        }
        if (_editing.value) {
            _editing.value = false
            scope.launch {
                try {
                    connection.request(Request.TurnEdit(session, text.trim()))
                } catch (e: Exception) {
                    publish(problem = e.message)
                }
            }
            return
        }
        scope.launch {
            val typed = SlashCommands.parse(text)
            if (typed != null) {
                if (_commands.value.isEmpty()) loadCommands()
                if (SlashCommands.runsOnEngine(typed, _commands.value)) {
                    run(typed)
                    return@launch
                }
            }
            try {
                connection.request(Request.TurnSend(session, text.trim()))
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    private suspend fun run(typed: SlashCommands.Typed) {
        val shownAs = "/${typed.name}" + if (typed.args.isEmpty()) "" else " ${typed.args}"
        val markdown = try {
            (connection.request(Request.CommandRun(session, typed.name, typed.args)) as? Reply.CommandResult)?.markdown ?: ""
        } catch (e: Exception) {
            e.message ?: "That command didn't work."
        }
        synchronized(model) { model.commandResult(shownAs, markdown) }
        publish()
    }

    /** Move the session to another folder (`/folder <path>`). */
    fun moveTo(folder: String) {
        scope.launch { run(SlashCommands.Typed("folder", folder)) }
    }

    /** Answer a permission card. If another device answered first, its answer stands. */
    fun answer(request: String, choice: String) {
        scope.launch {
            try {
                connection.request(Request.PermissionAnswer(request, choice))
            } catch (e: RequestFailed) {
                if (e.code != ALREADY_ANSWERED) publish(problem = e.message)
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    /** Answer a question card, one answer per question. If another device answered first, its answers stand. */
    fun answerQuestions(question: String, answers: List<String>) {
        scope.launch {
            try {
                connection.request(Request.QuestionAnswer(question, answers))
            } catch (e: RequestFailed) {
                if (e.code != ALREADY_ANSWERED) publish(problem = e.message)
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    /** Approve or turn down a plan. If another device decided first, its decision stands. */
    fun decide(plan: String, approve: Boolean, autoEdits: Boolean = false) {
        scope.launch {
            try {
                connection.request(Request.PlanDecide(plan, approve, autoEdits))
            } catch (e: RequestFailed) {
                if (e.code != ALREADY_ANSWERED) publish(problem = e.message)
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    fun stop() {
        scope.launch { runCatching { connection.request(Request.TurnInterrupt(session)) } }
    }

    /**
     * Change the mode; the header changes when the engine confirms it. For Full auto, [until] and
     * [minutes] say how long it lasts (the fingerprint check comes before this).
     */
    fun setMode(newMode: String, until: String? = null, minutes: Int? = null) {
        synchronized(model) { model.keepMode() }
        publish()
        scope.launch {
            try {
                connection.request(Request.SessionSetMode(session, newMode, until, minutes))
            } catch (e: Exception) {
                publish(problem = e.message)
            }
        }
    }

    /** Stay in the mode after the offer to go back to Ask. */
    fun keepMode() {
        synchronized(model) { model.keepMode() }
        publish()
    }

    fun dismissProblem() = publish(problem = null)

    fun close() = connection.sendLater(Request.SessionDetach(session))

    private companion object {
        /** The session's state when Thrall isn't doing anything. */
        const val IDLE = "idle"
        /** The engine's code when someone else answered a card first. */
        const val ALREADY_ANSWERED = "already_answered"
        /** Finished replies held for a slow listener. */
        const val REPLIES_BUFFER = 8
    }
}