package app.thrall

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import app.thrall.ui.AboutActions
import app.thrall.ui.AccountScreen
import app.thrall.ui.ChatExtras
import app.thrall.ui.GroupsScreen
import app.thrall.ui.HomeScreen
import app.thrall.ui.MemoryScreen
import app.thrall.ui.MoreScreen
import app.thrall.ui.SettingsScreen
import app.thrall.ui.ShareToScreen
import android.widget.Toast
import app.thrall.lock.AppLock
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.LaunchedEffect
import app.thrall.notify.Notifier
import app.thrall.notify.StayConnected
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.LocalContext
import app.thrall.data.UpdateState
import app.thrall.ui.LockScreen
import app.thrall.ui.PhoneMenu
import app.thrall.ui.SessionActions
import kotlinx.coroutines.launch
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.thrall.data.FileSource
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import app.thrall.data.LinkState
import app.thrall.ui.AttachActions
import app.thrall.ui.ChatModelView
import app.thrall.ui.ChatScreen
import app.thrall.ui.FileActions
import app.thrall.ui.FileScreen
import app.thrall.ui.FolderPicker
import app.thrall.ui.FolderScreen
import app.thrall.ui.LocalTokens
import app.thrall.ui.PairScreen
import app.thrall.ui.SessionsModel
import app.thrall.ui.SessionsScreen
import app.thrall.ui.StatusStrip
import app.thrall.ui.ThrallTheme

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val app = application as ThrallApp
        opened(intent)
        setContent { ThrallTheme { Root(app) } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /**
     * Opened from a notification (show its chat), by sharing from another app (pick a chat for it),
     * or from the widget or quick tile (the last chat, listening).
     */
    private fun opened(intent: Intent?) {
        intent ?: return
        val app = application as ThrallApp
        intent.getStringExtra(Notifier.EXTRA_SESSION)?.takeIf { it.isNotEmpty() }?.let { app.openChat.value = it }
        if (intent.getBooleanExtra(EXTRA_DICTATE, false)) app.dictate.value = true
        @Suppress("DEPRECATION")
        val uris = when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))
            Intent.ACTION_SEND_MULTIPLE -> intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty()
            else -> return
        }
        val text = listOfNotNull(intent.getStringExtra(Intent.EXTRA_SUBJECT), intent.getStringExtra(Intent.EXTRA_TEXT))
            .filter { it.isNotBlank() }.distinct().joinToString("\n")
        if (uris.isNotEmpty() || text.isNotBlank()) app.incoming.value = Shared(uris, text)
    }

    companion object {
        /** Start dictating in the last chat (the widget and quick tile). */
        const val EXTRA_DICTATE = "app.thrall.dictate"
    }

    override fun onStart() {
        super.onStart()
        val app = application as ThrallApp
        app.visible = true
        app.lock.back(this)
        // Back in front: make sure the link works (reconnect now rather than wait out a retry or a
        // link that died quietly while the phone slept), and look for a newer app.
        app.connection.checkLink()
        app.checkForUpdate()
        // Staying connected needs its notification allowed (asked once; the person can say no).
        val paired = app.connection.state.value !is LinkState.NotPaired
        if (app.stay.enabled && paired) {
            // (allowed() is always true before Android 13, which had no such permission.)
            if (!Notifier.allowed(this)) {
                askNotifications.launch(android.Manifest.permission.POST_NOTIFICATIONS)
            }
            StayConnected.sync(this, true)
        }
    }

    override fun onStop() {
        super.onStop()
        val app = application as ThrallApp
        app.visible = false
        app.lock.away()
    }
}

@Composable
private fun Root(app: ThrallApp) {
    val t = LocalTokens.current
    val state by app.connection.state.collectAsStateWithLifecycle()
    val lock by app.lock.locked.collectAsStateWithLifecycle()
    val activity = LocalActivity.current ?: return
    Column(Modifier.fillMaxSize().background(t.bg)) {
        if (lock.locked) {
            Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                LockScreen(lock.problem, onUnlock = { app.lock.unlock(activity) })
            }
            return@Column
        }
        when (val s = state) {
            LinkState.NotPaired -> PairScreen(app.connection)
            is LinkState.Refused -> PairScreen(app.connection, note = s.reason)
            else -> Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                StatusStrip(s, onRetry = app.connection::retryNow)
                Main(app, connected = s is LinkState.Connected)
            }
        }
    }
}

/** Where the person is: the list, a conversation, its files, or one file. */
private sealed interface Screen {
    data object Sessions : Screen
    data class Chat(val session: String) : Screen
    data class Folder(val session: String, val title: String, val path: String) : Screen
    data class File(val source: FileSource) : Screen
    /** Pick a folder: for a new chat ([session] null) or to move the open one. */
    data class PickFolder(val session: String?) : Screen
    /** Pick the chat something shared goes to. */
    data object ShareTo : Screen
    /** Groups, Home and check-ins, memory or settings (the ⋮ menu). */
    data class More(val which: MoreScreen) : Screen
}

@Composable
private fun Main(app: ThrallApp, connected: Boolean) {
    var stack by remember { mutableStateOf(listOf<Screen>(Screen.Sessions)) }
    val push: (Screen) -> Unit = { stack = stack + it }
    val pop: () -> Unit = { if (stack.size > 1) stack = stack.dropLast(1) }
    // A notification asked for a chat: open it over the list.
    val openChat by app.openChat.collectAsStateWithLifecycle()
    LaunchedEffect(openChat) {
        openChat?.let {
            stack = listOf(Screen.Sessions, Screen.Chat(it))
            app.openChat.value = null
        }
    }
    // Shared from another app (or sent on from a chat): pick the chat it goes to.
    val incoming by app.incoming.collectAsStateWithLifecycle()
    LaunchedEffect(incoming) {
        if (incoming != null && stack.last() != Screen.ShareTo) stack = listOf(Screen.Sessions, Screen.ShareTo)
    }
    // The widget or quick tile: the last chat (or Home), listening.
    val dictate by app.dictate.collectAsStateWithLifecycle()
    var dictateIn by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(dictate, connected) {
        if (!dictate) return@LaunchedEffect
        val target = app.lastChat
            ?: if (connected) runCatching { (app.connection.request(Request.HomeGet) as? Reply.HomeShown)?.page?.session }.getOrNull() else null
        if (target == null && !connected) return@LaunchedEffect
        app.dictate.value = false
        if (target != null) {
            dictateIn = target
            stack = listOf(Screen.Sessions, Screen.Chat(target))
        }
    }
    // Whose phone this is. Given to someone else on the computer, it starts again from the list.
    val me by app.connection.me.collectAsStateWithLifecycle()
    val owner = me?.owner ?: true
    var lastPerson by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(me?.id) {
        val id = me?.id ?: return@LaunchedEffect
        if (lastPerson != null && lastPerson != id) {
            app.lastChat = null
            stack = listOf(Screen.Sessions)
        }
        lastPerson = id
    }
    if (stack.size > 1) BackHandler(onBack = pop)
    // The open conversation lives while it's anywhere in the stack (looking at its files keeps it).
    val chatSession = stack.filterIsInstance<Screen.Chat>().lastOrNull()?.session
    LaunchedEffect(chatSession) { chatSession?.let { app.lastChat = it } }
    // Each conversation gets its own scope, ended (and detached) when it leaves the stack.
    val chatScope = remember(chatSession) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val chat = remember(chatSession) {
        chatSession?.let { ChatModelView(app.connection, it, chatScope, app.uploads, app.phoneFiles, app.outbox) }
    }
    DisposableEffect(chat) {
        onDispose {
            chat?.close()
            chatScope.cancel()
        }
    }
    // Things shared into this chat go into its message bar.
    val handoff by app.handoff.collectAsStateWithLifecycle()
    LaunchedEffect(chat, handoff) {
        val (session, shared) = handoff ?: return@LaunchedEffect
        if (chat == null || session != chatSession) return@LaunchedEffect
        if (shared.uris.isNotEmpty()) chat.attach(shared.uris)
        chat.setDraft(shared.text)
        app.handoff.value = null
    }
    val files = remember(chatSession) {
        chatSession?.let { s ->
            FileActions(
                app.downloads,
                s,
                open = { source -> push(Screen.File(source)) },
                showIn = { path ->
                    val folder = path.replace('\\', '/').substringBeforeLast('/', "")
                    push(Screen.Folder(s, "Files", folder))
                },
                forward = { file, _ -> app.incoming.value = Shared(listOf(app.phoneFiles.uriOf(file)), "") },
            )
        }
    }
    /** Give what was shared to [session] and open it. */
    val deliver: (String) -> Unit = { session ->
        app.incoming.value?.let { app.handoff.value = session to it }
        app.incoming.value = null
        stack = listOf(Screen.Sessions, Screen.Chat(session))
    }
    when (val screen = stack.last()) {
        Screen.Sessions -> {
            val model: SessionsModel = viewModel()
            val shown by model.shown.collectAsStateWithLifecycle()
            val update by app.updater.state.collectAsStateWithLifecycle()
            val lock by app.lock.locked.collectAsStateWithLifecycle()
            val context = LocalContext.current
            val activity = LocalActivity.current ?: return
            val scope = rememberCoroutineScope()
            // Read again after the lock's switch is flipped (it changes only after unlocking).
            var lockOn by remember(lock) { mutableStateOf(app.lock.enabled) }
            var stayOn by remember { mutableStateOf(app.stay.enabled) }
            SessionsScreen(
                shown = shown,
                onOpen = { push(Screen.Chat(it.session)) },
                onNew = { push(Screen.PickFolder(null)) },
                onRefresh = model::refresh,
                actions = SessionActions(
                    onRename = model::rename,
                    onPin = model::pin,
                    onArchive = model::archive,
                    onDelete = model::delete,
                    onShowArchived = model::showArchived,
                    onMove = model::move,
                    onNewGroup = model::newGroup,
                    onRenameGroup = model::renameGroup,
                    onDeleteGroup = model::deleteGroup,
                ),
                menu = PhoneMenu(
                    lockOn = lockOn,
                    onLock = { on ->
                        app.lock.setEnabled(activity, on) { problem ->
                            lockOn = app.lock.enabled
                            problem?.let { Toast.makeText(context, it, Toast.LENGTH_LONG).show() }
                        }
                    },
                    stayOn = stayOn,
                    onStay = { on ->
                        app.stay.enabled = on
                        stayOn = on
                        StayConnected.sync(context, on)
                        val note = if (on) "Thrall stays connected while the app is closed." else "Notifications stop when the app is closed."
                        Toast.makeText(context, note, Toast.LENGTH_SHORT).show()
                    },
                    update = update,
                    onUpdate = {
                        val release = when (val u = update) {
                            is UpdateState.Available -> u.release
                            is UpdateState.NeedsPermission -> u.release
                            is UpdateState.Failed -> u.release
                            else -> null
                        }
                        if (release != null) {
                            if (update is UpdateState.NeedsPermission && !app.updater.allowed()) {
                                context.startActivity(app.updater.permissionIntent())
                            } else {
                                scope.launch { app.updater.install(release) }
                            }
                        }
                    },
                    onUnpair = {
                        StayConnected.sync(context, false)
                        app.connection.unpair()
                    },
                    onOpen = { push(Screen.More(it)) },
                    signedInAs = me?.name,
                    owner = owner,
                ),
            )
        }
        is Screen.More -> when (screen.which) {
            MoreScreen.Groups -> {
                val model: SessionsModel = viewModel()
                val shown by model.shown.collectAsStateWithLifecycle()
                GroupsScreen(shown, model, onBack = pop)
            }
            MoreScreen.Home -> HomeScreen(app.connection, onBack = pop)
            MoreScreen.Memory -> MemoryScreen(app.connection, onBack = pop)
            MoreScreen.Account -> AccountScreen(app.connection, onBack = pop)
            MoreScreen.Settings -> {
                val link by app.connection.state.collectAsStateWithLifecycle()
                val context = LocalContext.current
                val scope = rememberCoroutineScope()
                val about = AboutActions(
                    appVersion = BuildConfig.VERSION_NAME,
                    engineVersion = (link as? LinkState.Connected)?.engineVersion,
                    check = app.updater::check,
                    update = {
                        val release = (app.updater.state.value as? UpdateState.Available)?.release
                        when {
                            release == null -> Unit
                            !app.updater.allowed() -> context.startActivity(app.updater.permissionIntent())
                            else -> scope.launch { app.updater.install(release) }
                        }
                    },
                )
                SettingsScreen(app.connection, owner, about, onBack = pop)
            }
        }
        is Screen.Chat -> {
            val model = chat ?: return
            val shown by model.shown.collectAsStateWithLifecycle()
            val commands by model.commands.collectAsStateWithLifecycle()
            val attachments by model.attachments.collectAsStateWithLifecycle()
            val queued by model.queued.collectAsStateWithLifecycle()
            val draft by model.draft.collectAsStateWithLifecycle()
            val editing by model.editing.collectAsStateWithLifecycle()
            val chatActivity = LocalActivity.current ?: return
            val chatContext = LocalContext.current
            ChatScreen(
                shown = shown,
                connected = connected,
                onBack = pop,
                onSend = model::send,
                onStop = model::stop,
                onDismissProblem = model::dismissProblem,
                onAnswer = model::answer,
                onDecide = model::decide,
                files = files ?: return,
                onFiles = { push(Screen.Folder(screen.session, shown.title, "")) },
                commands = commands,
                onFolder = { push(Screen.PickFolder(screen.session)) },
                replies = model.replies,
                attach = AttachActions(
                    picked = attachments,
                    onAttach = model::attach,
                    onRemove = model::remove,
                    onRetry = model::retry,
                    newPhoto = app.phoneFiles::newPhoto,
                    uriOf = model::uriOf,
                ),
                onMode = model::setMode,
                extras = ChatExtras(
                    queued = queued,
                    onUnqueue = model::unqueue,
                    draft = draft,
                    onDraftTaken = model::draftTaken,
                    dictateNow = dictateIn == screen.session,
                    onDictateStarted = { dictateIn = null },
                    editing = editing,
                    onEdit = model::startEdit,
                    onCancelEdit = model::cancelEdit,
                    onRetry = model::retry,
                    previewRewind = model::previewRewind,
                    onRewind = model::rewind,
                    owner = owner,
                    onFullAuto = { until, minutes ->
                        // A third check after the phone lock and the app lock.
                        app.lock.confirm(chatActivity, "Switch on Full auto") { problem ->
                            when (problem) {
                                null -> model.setMode("full_auto", until, minutes)
                                AppLock.CANCELLED -> Unit
                                else -> Toast.makeText(chatContext, problem, Toast.LENGTH_LONG).show()
                            }
                        }
                    },
                    onKeepMode = model::keepMode,
                ),
            )
        }
        Screen.ShareTo -> {
            val sessions: SessionsModel = viewModel()
            val shown by sessions.shown.collectAsStateWithLifecycle()
            val shared = incoming
            if (shared == null) {
                LaunchedEffect(Unit) { stack = listOf(Screen.Sessions) }
                return
            }
            ShareToScreen(
                shared = shared,
                shown = shown,
                onPick = { deliver(it.session) },
                onNew = { push(Screen.PickFolder(null)) },
                onCancel = {
                    app.incoming.value = null
                    stack = listOf(Screen.Sessions)
                },
            )
        }
        is Screen.PickFolder -> {
            val sessions: SessionsModel = viewModel()
            var pickError by remember(screen) { mutableStateOf<String?>(null) }
            val moving = screen.session?.let { chat }
            val movingShown = moving?.shown?.collectAsStateWithLifecycle()
            FolderPicker(
                connection = app.connection,
                title = if (moving != null) "Move this chat to…" else "Where should this chat work?",
                current = movingShown?.value?.workspace,
                problem = pickError,
                onPick = { folder ->
                    pickError = null
                    if (moving != null) {
                        moving.moveTo(folder)
                        pop()
                    } else {
                        sessions.newChat(
                            folder,
                            opened = { s ->
                                // A new chat for something shared: it goes in there.
                                if (app.incoming.value != null) deliver(s.session) else stack = stack.dropLast(1) + Screen.Chat(s.session)
                            },
                            failed = { pickError = it },
                        )
                    }
                },
                onBack = pop,
            )
        }
        is Screen.Folder -> FolderScreen(
            connection = app.connection,
            session = screen.session,
            title = screen.title,
            path = screen.path,
            onOpen = { push(Screen.File(FileSource.Folder(screen.session, it))) },
            onFolder = { push(screen.copy(path = it)) },
            onBack = pop,
        )
        is Screen.File -> FileScreen(app.downloads, screen.source, onBack = pop)
    }
}