package app.thrall.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.chat.ChatItem
import app.thrall.chat.ChatModel
import app.thrall.chat.SlashCommands
import app.thrall.protocol.CommandItem
import app.thrall.data.Downloads
import app.thrall.data.FileKind
import app.thrall.data.FileKinds
import app.thrall.data.FileSource
import app.thrall.protocol.FileRef
import app.thrall.protocol.Diff
import app.thrall.protocol.Reply
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import android.Manifest
import android.content.pm.PackageManager
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.IconButton
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import app.thrall.voice.Dictation
import app.thrall.voice.Speaker
import app.thrall.voice.SpeechText
import app.thrall.voice.VoiceSettings
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.activity.result.PickVisualMediaRequest
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.TextStyle
import app.thrall.chat.Attachment
import app.thrall.chat.Attachments

/** Permission modes anyone can pick on the phone; Full auto is added for the owner. */
private val MODES = listOf("ask" to "Ask", "auto_edits" to "Auto-accept edits", "plan" to "Plan")

/** Short names for the chip. */
private val MODE_CHIP = mapOf("ask" to "Ask", "auto_edits" to "Auto edits", "plan" to "Plan", "full_auto" to "Full auto")

/** What each mode lets Thrall do without asking (the engine's wording), shown when it's picked. */
private val MODE_COVERS = mapOf(
    "ask" to "Thrall asks before every file edit and every command.",
    "auto_edits" to "File edits in the folder go ahead without asking; commands still ask.",
    "plan" to "Thrall only reads and plans; nothing changes until you approve a plan.",
    "full_auto" to "Edits and commands go ahead without asking, risky ones too (restarts, sending and deleting outside the folder still ask).",
)

/** How long Full auto can last: label, `until`, minutes. */
private val FULL_AUTO_FOR = listOf(
    Triple("Until this task ends", ChatModel.UNTIL_TASK_END, null),
    Triple("For 15 minutes", ChatModel.UNTIL_TIME, 15),
    Triple("For 1 hour", ChatModel.UNTIL_TIME, 60),
    Triple("Until I switch it off", ChatModel.UNTIL_MANUAL, null),
)

/** How much Thrall may do without asking, for this chat; tap to change. */
@Composable
private fun ModeChip(mode: String, owner: Boolean, onMode: (String) -> Unit, onFullAuto: () -> Unit) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var open by remember { mutableStateOf(false) }
    val modes = if (owner) MODES + ("full_auto" to "Full auto…") else MODES
    Box {
        Text(
            (MODE_CHIP[mode] ?: mode) + " ▾",
            color = when (mode) {
                "ask" -> t.inkMuted
                "full_auto" -> t.danger
                else -> t.accent
            },
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            modifier = Modifier
                .background(t.surface, RoundedCornerShape(999.dp))
                .clickable { open = true }
                .padding(horizontal = 10.dp, vertical = 4.dp),
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for ((id, label) in modes) {
                DropdownMenuItem(
                    text = { Text(if (id == mode) "✓ $label" else label) },
                    onClick = {
                        open = false
                        when {
                            id == "full_auto" -> onFullAuto()
                            id != mode -> {
                                onMode(id)
                                Toast.makeText(context, MODE_COVERS[id], Toast.LENGTH_LONG).show()
                            }
                        }
                    },
                )
            }
        }
    }
}

/** Before Full auto: what it means and how long it lasts; the fingerprint check comes next. */
@Composable
private fun FullAutoDialog(onPick: (until: String, minutes: Int?) -> Unit, onCancel: () -> Unit) {
    val t = LocalTokens.current
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Switch on Full auto?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(MODE_COVERS.getValue("full_auto"), fontSize = 14.sp)
                Text("Only in this chat. You'll confirm with your fingerprint, face or screen lock.", color = t.inkMuted, fontSize = 13.sp)
                for ((label, until, minutes) in FULL_AUTO_FOR) {
                    OutlinedButton(onClick = { onPick(until, minutes) }, modifier = Modifier.fillMaxWidth()) {
                        Text(label, color = t.danger)
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } },
    )
}

/** While Full auto is on: a red bar with how long it lasts and Stop. After a task in a mode that
 * doesn't ask: the offer to go back to Ask. */
@Composable
private fun ModeBar(shown: ChatModelView.Shown, onMode: (String) -> Unit, onKeep: () -> Unit) {
    val t = LocalTokens.current
    if (shown.mode == "full_auto") {
        val lasts = when (shown.modeLimit?.until) {
            ChatModel.UNTIL_MANUAL -> "until you stop it"
            ChatModel.UNTIL_TIME -> shown.modeLimit.endsAt?.let {
                "until " + java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(it))
            } ?: "for a while"
            else -> "until this task ends"
        }
        Row(
            Modifier.fillMaxWidth().background(t.danger).padding(horizontal = 16.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Full auto is on $lasts: no questions before edits or commands.", color = t.bg, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            TextButton(onClick = { onMode("ask") }) { Text("Stop", color = t.bg, fontWeight = FontWeight.Bold) }
        }
    } else if (shown.offerAsk) {
        Row(
            Modifier.fillMaxWidth().background(t.surface).padding(horizontal = 16.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Still on ${MODE_CHIP[shown.mode] ?: shown.mode}. Back to Ask?", color = t.ink, fontSize = 13.sp, modifier = Modifier.weight(1f))
            TextButton(onClick = { onMode("ask") }) { Text("Ask", color = t.accent) }
            TextButton(onClick = onKeep) { Text("Keep", color = t.inkMuted) }
        }
    }
}

@Composable
fun ChatScreen(
    shown: ChatModelView.Shown,
    connected: Boolean,
    onBack: () -> Unit,
    onSend: (String) -> Unit,
    onStop: () -> Unit,
    onDismissProblem: () -> Unit,
    onAnswer: (request: String, choice: String) -> Unit,
    onDecide: (plan: String, approve: Boolean, autoEdits: Boolean) -> Unit,
    files: FileActions,
    onFiles: () -> Unit,
    commands: List<CommandItem>,
    onFolder: () -> Unit,
    replies: Flow<ChatItem.Message>,
    attach: AttachActions,
    onMode: (String) -> Unit,
    extras: ChatExtras,
) {
    val t = LocalTokens.current
    val list = rememberLazyListState()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    // Rewinding: what it would do, shown before it's done.
    var rewinding by remember { mutableStateOf<Reply.CheckpointShown?>(null) }
    rewinding?.let { preview ->
        RewindDialog(preview, onDone = { files ->
            if (files != null) extras.onRewind(preview.turn, files)
            rewinding = null
        })
    }
    // Reading aloud: one voice for the chat, stopped when the chat closes.
    val speaker = remember { Speaker(context) }
    DisposableEffect(speaker) { onDispose { speaker.shutdown() } }
    val speaking by speaker.speaking.collectAsState()
    val settings = remember { VoiceSettings(context) }
    var readReplies by remember { mutableStateOf(settings.readReplies) }
    val reading by rememberUpdatedState(readReplies)
    LaunchedEffect(replies) {
        replies.collect { if (reading) speaker.speak(it.key, it.text, after = true) }
    }
    // Follow the conversation as it grows (including text streaming into the last row).
    val last = shown.rows.lastOrNull()
    LaunchedEffect(shown.rows.size, (last as? ChatItem.Message)?.text?.length) {
        if (shown.rows.isNotEmpty()) list.animateScrollToItem(shown.rows.lastIndex)
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        // The folder the chat works in; tap it to move the chat somewhere else.
        val folder = shown.workspace?.let { "📁 ${folderName(it)} · change" } ?: "📁 No folder picked · pick one"
        var askingFullAuto by remember { mutableStateOf(false) }
        if (askingFullAuto) {
            FullAutoDialog(
                onPick = { until, minutes -> askingFullAuto = false; extras.onFullAuto(until, minutes) },
                onCancel = { askingFullAuto = false },
            )
        }
        Header(title = shown.title.ifBlank { "New session" }, subtitle = folder, onBack = onBack, onSubtitle = onFolder) {
            ModeChip(shown.mode, extras.owner, onMode, onFullAuto = { askingFullAuto = true })
            when (shown.state) {
                "working" -> Badge("Working", t.warn)
                "waiting_for_you" -> Badge("Needs you", t.accent)
            }
            // Read new replies aloud, or not (kept on this phone).
            TextButton(onClick = {
                readReplies = !readReplies
                settings.readReplies = readReplies
                if (!readReplies) speaker.stop()
                Toast.makeText(context, if (readReplies) "New replies will be read aloud." else "Replies won't be read aloud.", Toast.LENGTH_SHORT).show()
            }) { Text(if (readReplies) "🔊" else "🔇", color = if (readReplies) t.accent else t.inkMuted) }
            TextButton(onClick = onFiles) { Text("Files", color = t.accent) }
        }
        ModeBar(shown, onMode, extras.onKeepMode)
        LazyColumn(
            state = list,
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val lastMine = shown.rows.lastOrNull { it is ChatItem.Message && it.fromUser }?.key
            items(shown.rows, key = { it.key }) { row ->
                val menu = (row as? ChatItem.Message)?.takeIf { it.fromUser && connected }?.let { m ->
                    MessageMenu(
                        last = m.key == lastMine,
                        idle = shown.state == "idle",
                        onEdit = { extras.onEdit(m.text) },
                        onRetry = extras.onRetry,
                        onRewind = {
                            val turn = m.turn ?: return@MessageMenu
                            scope.launch { rewinding = extras.previewRewind(turn) }
                        },
                    )
                }
                ChatRow(row, connected, onAnswer, onDecide, files, shown.workspace, speaking == row.key, menu) { m -> speaker.toggle(m.key, m.text) }
            }
            // Typed while offline: shown as waiting, sent when Thrall's back (✕ takes one back).
            items(extras.queued, key = { it.id }) { q ->
                Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End) {
                    Text(
                        q.text,
                        color = t.inkMuted,
                        fontSize = 15.sp,
                        modifier = Modifier
                            .widthIn(max = 320.dp)
                            .border(1.dp, t.line, RoundedCornerShape(14.dp))
                            .padding(horizontal = 14.dp, vertical = 10.dp),
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("Waiting to send: it goes when Thrall's back", color = t.inkMuted, fontSize = 11.sp)
                        TextButton(onClick = { extras.onUnqueue(q.id) }) { Text("✕", color = t.inkMuted) }
                    }
                }
            }
        }
        shown.problem?.let {
            Row(
                Modifier.fillMaxWidth().background(t.surface).padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(it, color = t.danger, fontSize = 13.sp, modifier = Modifier.weight(1f))
                TextButton(onClick = onDismissProblem) { Text("OK") }
            }
        }
        if (extras.editing) {
            Row(
                Modifier.fillMaxWidth().background(t.accentSoft).padding(horizontal = 16.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Editing your last message: sending replaces it.", color = t.ink, fontSize = 12.sp, modifier = Modifier.weight(1f))
                TextButton(onClick = extras.onCancelEdit) { Text("Cancel") }
            }
        }
        Composer(
            working = shown.state != "idle",
            connected = connected,
            commands = commands,
            attach = attach,
            onSend = onSend,
            onStop = onStop,
            draft = extras.draft,
            onDraftTaken = extras.onDraftTaken,
            dictateNow = extras.dictateNow,
            onDictateStarted = extras.onDictateStarted,
        )
    }
}

/**
 * The rest of what a chat shows: messages waiting to go while offline, text shared in for the
 * message bar, and starting dictation straight away (the widget or quick tile).
 */
class ChatExtras(
    val queued: List<app.thrall.data.Queued>,
    val onUnqueue: (String) -> Unit,
    val draft: String?,
    val onDraftTaken: () -> Unit,
    val dictateNow: Boolean,
    val onDictateStarted: () -> Unit,
    /** Changing the last message: the next send replaces it. */
    val editing: Boolean = false,
    val onEdit: (String) -> Unit = {},
    val onCancelEdit: () -> Unit = {},
    val onRetry: () -> Unit = {},
    val previewRewind: suspend (String) -> Reply.CheckpointShown? = { null },
    val onRewind: (turn: String, files: Boolean) -> Unit = { _, _ -> },
    /** The phone is the owner's: Full auto is offered. */
    val owner: Boolean = false,
    /** Full auto was picked with how long it lasts; check the fingerprint, then switch it on. */
    val onFullAuto: (until: String, minutes: Int?) -> Unit = { _, _ -> },
    /** The offer to go back to Ask was turned down. */
    val onKeepMode: () -> Unit = {},
)

/**
 * Before rewinding: the conversation goes back to before this message, and Thrall's file changes
 * since can be put back too (files someone else changed since are named). [onDone] gets whether
 * to put files back, or null to leave it.
 */
@Composable
private fun RewindDialog(preview: Reply.CheckpointShown, onDone: (Boolean?) -> Unit) {
    val t = LocalTokens.current
    val restorable = preview.files.filter { it.action != "unavailable" }
    var files by remember(preview) { mutableStateOf(restorable.isNotEmpty()) }
    androidx.compose.material3.AlertDialog(
        onDismissRequest = { onDone(null) },
        title = { Text("Rewind to before this message?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("This message and everything after it are taken back; the message goes back in the box.", fontSize = 14.sp)
                if (restorable.isNotEmpty()) {
                    Row(Modifier.clickable { files = !files }, verticalAlignment = Alignment.CenterVertically) {
                        androidx.compose.material3.Checkbox(checked = files, onCheckedChange = { files = it })
                        Text("Also put back the files Thrall changed (${restorable.size})", fontSize = 14.sp)
                    }
                    for (f in restorable.take(REWIND_FILES_SHOWN)) {
                        val what = if (f.action == "delete") "removed (Thrall made it)" else "put back"
                        Text(
                            "• ${f.path}: $what" + if (f.changed) ", changed since by someone else" else "",
                            color = if (f.changed) t.warn else t.inkMuted,
                            fontSize = 12.sp,
                        )
                    }
                    if (restorable.size > REWIND_FILES_SHOWN) Text("…and ${restorable.size - REWIND_FILES_SHOWN} more", color = t.inkMuted, fontSize = 12.sp)
                }
                val unavailable = preview.files.count { it.action == "unavailable" }
                if (unavailable > 0) Text("$unavailable file(s) can't be put back (too big to keep a copy, or changed by a command).", color = t.inkMuted, fontSize = 12.sp)
            }
        },
        confirmButton = { TextButton(onClick = { onDone(files) }) { Text("Rewind", color = t.danger) } },
        dismissButton = { TextButton(onClick = { onDone(null) }) { Text("Cancel") } },
    )
}

/** Files named in the rewind dialog before "…and N more". */
private const val REWIND_FILES_SHOWN = 8

/** What holding down one of your messages offers: the last can be edited or sent again; any can be rewound to. */
private class MessageMenu(val last: Boolean, val idle: Boolean, val onEdit: () -> Unit, val onRetry: () -> Unit, val onRewind: () -> Unit)

@Composable
private fun ChatRow(
    item: ChatItem,
    connected: Boolean,
    onAnswer: (String, String) -> Unit,
    onDecide: (String, Boolean, Boolean) -> Unit,
    files: FileActions,
    folder: String?,
    beingRead: Boolean,
    menu: MessageMenu?,
    onRead: (ChatItem.Message) -> Unit,
) {
    val t = LocalTokens.current
    val openPath: (String) -> Unit = { files.open(FileSource.Folder(files.session, it)) }
    val pathOf: (String) -> String? = { FileKinds.pathIn(it, folder) }
    when (item) {
        is ChatItem.Message -> if (item.fromUser) {
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                if (item.text.isNotBlank()) {
                    var open by remember(item.key) { mutableStateOf(false) }
                    Box {
                        Text(
                            item.text,
                            color = t.ink,
                            fontSize = 15.sp,
                            modifier = Modifier
                                .widthIn(max = 320.dp)
                                .background(t.accentSoft, RoundedCornerShape(14.dp))
                                .combinedClickable(enabled = menu != null, onClick = {}, onLongClick = { open = true })
                                .padding(horizontal = 14.dp, vertical = 10.dp),
                        )
                        if (menu != null) {
                            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                                if (menu.last && menu.idle) {
                                    DropdownMenuItem(text = { Text("Edit and send again") }, onClick = { open = false; menu.onEdit() })
                                    DropdownMenuItem(text = { Text("Send again") }, onClick = { open = false; menu.onRetry() })
                                }
                                if (menu.idle) {
                                    DropdownMenuItem(text = { Text("Rewind to before this…") }, onClick = { open = false; menu.onRewind() })
                                } else {
                                    DropdownMenuItem(text = { Text("Wait for Thrall to finish to change this", color = t.inkMuted) }, onClick = { open = false })
                                }
                            }
                        }
                    }
                }
                Attached(item.files, files)
            }
        } else {
            if (item.text.isEmpty() && item.streaming) {
                Text("…", color = t.inkMuted)
            } else Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Markdown(item.text + if (item.streaming) " ▍" else "", onPath = openPath, pathOf = pathOf)
                Attached(item.files, files)
                if (!item.streaming) {
                    Text(
                        if (beingRead) "■ Stop reading" else "🔊 Read aloud",
                        color = if (beingRead) t.accent else t.inkMuted,
                        fontSize = 12.sp,
                        modifier = Modifier.clickable { onRead(item) }.padding(vertical = 4.dp),
                    )
                }
            }
        }
        is ChatItem.Step -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Step(item)
            Attached(item.files, files)
            item.changed?.takeIf { item.status == "ok" }?.let { ChangedFile(it, files) }
        }
        is ChatItem.Waiting -> Waiting(item, connected, onAnswer, onDecide)
        is ChatItem.Note -> Text(item.text, color = if (item.warning) t.warn else t.inkMuted, fontSize = 13.sp)
        is ChatItem.Command -> Column(
            Modifier
                .fillMaxWidth()
                .background(t.surface, RoundedCornerShape(10.dp))
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(item.command, color = t.accent, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
            Markdown(item.markdown, onPath = openPath, pathOf = pathOf)
        }
    }
}

@Composable
private fun Step(item: ChatItem.Step) {
    val t = LocalTokens.current
    var open by remember { mutableStateOf(false) }
    val (mark, color) = when (item.status) {
        "running" -> "○" to t.warn
        "ok" -> "✓" to t.ok
        // Ran to the end, but its exit code wasn't 0: Thrall's reply says whether it did the job.
        "warn" -> "!" to t.warn
        "denied" -> "⊘" to t.inkMuted
        "cancelled" -> "–" to t.inkMuted
        else -> "✕" to t.danger
    }
    Column(
        Modifier
            .fillMaxWidth()
            .background(t.surface, RoundedCornerShape(10.dp))
            .clickable(enabled = item.summary.isNotEmpty() || item.output.isNotEmpty()) { open = !open }
            .padding(horizontal = 12.dp, vertical = 8.dp)
            .animateContentSize(),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(mark, color = color, modifier = Modifier.padding(end = 8.dp))
            Text(item.title, color = t.ink, fontSize = 13.sp, maxLines = if (open) 6 else 1, overflow = TextOverflow.Ellipsis)
        }
        if (open) {
            if (item.summary.isNotEmpty()) Text(item.summary, color = t.inkMuted, fontSize = 12.sp)
            if (item.output.isNotEmpty()) {
                Text(item.output.takeLast(1_500), color = t.inkMuted, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
            }
        }
    }
}

@Composable
private fun Composer(
    working: Boolean,
    connected: Boolean,
    commands: List<CommandItem>,
    attach: AttachActions,
    onSend: (String) -> Unit,
    onStop: () -> Unit,
    draft: String?,
    onDraftTaken: () -> Unit,
    dictateNow: Boolean,
    onDictateStarted: () -> Unit,
) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var text by rememberSaveable { mutableStateOf("") }
    // Text shared in from another app joins whatever is already typed.
    LaunchedEffect(draft) {
        if (draft != null) {
            text = if (text.isBlank()) draft else text.trimEnd() + " " + draft
            onDraftTaken()
        }
    }
    // Dictation: what's heard is added after what was already typed.
    var listening by remember { mutableStateOf(false) }
    var before by remember { mutableStateOf("") }
    val dictation = remember {
        Dictation(
            context,
            onText = { heard -> text = if (before.isBlank()) SpeechText.dictated(heard) else before.trimEnd() + " " + heard },
            onEnd = { problem ->
                listening = false
                problem?.let { Toast.makeText(context, it, Toast.LENGTH_LONG).show() }
            },
        )
    }
    DisposableEffect(dictation) { onDispose { dictation.release() } }
    val listen = {
        before = text
        listening = true
        dictation.start()
    }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) listen() else Toast.makeText(context, "Thrall needs the microphone to take dictation.", Toast.LENGTH_LONG).show()
    }
    val startDictating = {
        when {
            listening -> Unit
            !dictation.available -> Toast.makeText(context, "This phone has no speech recognition.", Toast.LENGTH_LONG).show()
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED -> listen()
            else -> askMic.launch(Manifest.permission.RECORD_AUDIO)
        }
    }
    // Opened from the widget or quick tile: start listening straight away.
    LaunchedEffect(dictateNow) {
        if (dictateNow) {
            onDictateStarted()
            startDictating()
        }
    }
    // Typing `/`: the commands that match, tap one to fill it in.
    val offered = SlashCommands.suggestions(text, commands)
    if (offered.isNotEmpty()) {
        Column(Modifier.fillMaxWidth().background(t.surface).padding(horizontal = 8.dp, vertical = 4.dp)) {
            for (c in offered) {
                Row(
                    Modifier.fillMaxWidth().clickable { text = "/${c.name} " }.padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("/${c.name}", color = t.accent, fontFamily = FontFamily.Monospace, fontSize = 14.sp)
                    if (c.argsHint.isNotEmpty()) {
                        Text(" ${c.argsHint}", color = t.inkMuted, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
                    }
                    Text(
                        "  ${c.summary}",
                        color = t.inkMuted,
                        fontSize = 12.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
    if (attach.picked.isNotEmpty()) PickedStrip(attach)
    if (working && attach.picked.isNotEmpty()) {
        Text(
            "Files go once Thrall has finished what it's doing.",
            color = t.inkMuted,
            fontSize = 12.sp,
            modifier = Modifier.fillMaxWidth().background(t.surface).padding(horizontal = 16.dp, vertical = 2.dp),
        )
    }
    // Offline, a plain message can still be sent: it waits on the phone until Thrall's back.
    val canSend = if (connected) Attachments.canSend(text, attach.picked, working) else text.isNotBlank() && attach.picked.isEmpty()
    // One box across the whole width, its buttons inside: ＋ on the left, 🎤 and Send (or Stop) on
    // the right, kept on the bottom line as the text grows.
    Box(Modifier.fillMaxWidth().background(t.surface).padding(horizontal = 8.dp, vertical = 6.dp)) {
        Row(
            Modifier
                .fillMaxWidth()
                .border(1.dp, t.line, RoundedCornerShape(BAR_RADIUS))
                .background(t.bg, RoundedCornerShape(BAR_RADIUS))
                .padding(horizontal = 2.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            AttachMenu(attach, enabled = connected)
            BasicTextField(
                value = text,
                onValueChange = { text = it },
                textStyle = TextStyle(color = t.ink, fontSize = 16.sp),
                cursorBrush = SolidColor(t.accent),
                maxLines = 6,
                modifier = Modifier.weight(1f).padding(vertical = 13.dp),
                decorationBox = { field ->
                    Box {
                        if (text.isEmpty()) {
                            Text(
                                if (listening) "Listening…" else if (connected) "Message Thrall" else "Offline: it sends when Thrall's back",
                                color = t.inkMuted,
                                fontSize = 16.sp,
                            )
                        }
                        field()
                    }
                },
            )
            IconButton(onClick = { if (listening) dictation.stop() else startDictating() }) {
                Text(if (listening) "■" else "🎤", color = if (listening) t.danger else t.ink, fontSize = 20.sp)
            }
            if (working && text.isBlank() && attach.picked.isEmpty()) {
                IconButton(onClick = onStop) { Text("⏹", color = t.danger, fontSize = 20.sp) }
            } else {
                IconButton(
                    onClick = {
                        if (listening) dictation.stop()
                        onSend(text)
                        text = ""
                    },
                    enabled = canSend,
                ) { Text("➤", color = if (canSend) t.accent else t.inkMuted, fontSize = 22.sp) }
            }
        }
    }
}

/** What the message bar needs for files: the ones picked, and adding, removing and retrying them. */
class AttachActions(
    val picked: List<Attachment>,
    val onAttach: (List<Uri>) -> Unit,
    val onRemove: (Long) -> Unit,
    val onRetry: (Long) -> Unit,
    /** A fresh place for the camera to save a photo. */
    val newPhoto: () -> Uri,
    /** The phone's own link to a picked file, for its preview. */
    val uriOf: (Long) -> Uri? = { null },
)

/** ＋: take a photo, pick photos, or pick files. */
@Composable
private fun AttachMenu(attach: AttachActions, enabled: Boolean) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var open by remember { mutableStateOf(false) }
    var photo by rememberSaveable { mutableStateOf<Uri?>(null) }
    val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        if (taken) photo?.let { attach.onAttach(listOf(it)) }
    }
    val shoot = {
        val target = attach.newPhoto()
        photo = target
        camera.launch(target)
    }
    // The app holds the camera permission (for pairing QR codes), so the camera app needs it too.
    val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) shoot() else Toast.makeText(context, "Thrall needs the camera to take a photo.", Toast.LENGTH_LONG).show()
    }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(Attachments.MAX_FILES)) { uris ->
        if (uris.isNotEmpty()) attach.onAttach(uris)
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) attach.onAttach(uris)
    }
    Box {
        IconButton(onClick = { open = true }, enabled = enabled) {
            Text("＋", color = if (enabled) t.accent else t.inkMuted, fontSize = 22.sp)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Text("📷  Camera") }, onClick = {
                open = false
                if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                    shoot()
                } else {
                    askCamera.launch(Manifest.permission.CAMERA)
                }
            })
            DropdownMenuItem(text = { Text("🖼  Photos") }, onClick = {
                open = false
                photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
            })
            DropdownMenuItem(text = { Text("📄  Files") }, onClick = {
                open = false
                documents.launch(arrayOf(ANY_FILE))
            })
        }
    }
}

/** The files picked for the next message: a preview or chip each, ✕ to take it off, tap to retry. */
@Composable
private fun PickedStrip(attach: AttachActions) {
    val t = LocalTokens.current
    Column(Modifier.fillMaxWidth().background(t.surface).padding(start = 12.dp, end = 12.dp, top = 8.dp)) {
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (a in attach.picked) PickedItem(a, attach)
        }
        attach.picked.firstOrNull { it.problem != null }?.let { a ->
            Text(
                if (a.tooBig) a.problem.orEmpty() else "${a.problem} Tap it to try again.",
                color = t.danger,
                fontSize = 12.sp,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}

@Composable
private fun PickedItem(a: Attachment, attach: AttachActions) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var preview by remember(a.key) { mutableStateOf<ImageBitmap?>(null) }
    if (a.image) {
        LaunchedEffect(a.key) {
            val uri = attach.uriOf(a.key) ?: return@LaunchedEffect
            preview = withContext(Dispatchers.IO) {
                runCatching {
                    ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                        val scale = PREVIEW_PX.toFloat() / maxOf(info.size.width, info.size.height)
                        if (scale < 1f) decoder.setTargetSize((info.size.width * scale).toInt().coerceAtLeast(1), (info.size.height * scale).toInt().coerceAtLeast(1))
                    }.asImageBitmap()
                }.getOrNull()
            }
        }
    }
    val failed = a.problem != null
    Box(
        Modifier
            .size(PREVIEW_DP)
            .border(1.dp, if (failed) t.danger else t.line, RoundedCornerShape(10.dp))
            .background(t.bg, RoundedCornerShape(10.dp))
            .clickable(enabled = failed && !a.tooBig) { attach.onRetry(a.key) },
        contentAlignment = Alignment.Center,
    ) {
        val image = preview
        if (image != null) {
            Image(image, contentDescription = a.name, contentScale = ContentScale.Crop, modifier = Modifier.matchParentSize())
        } else {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(4.dp)) {
                Text(if (a.image) "🖼" else "📄", fontSize = 20.sp)
                Text(a.name, color = t.inkMuted, fontSize = 10.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        when {
            a.uploading -> CircularProgressIndicator(
                progress = { a.progress },
                color = t.accent,
                strokeWidth = 3.dp,
                modifier = Modifier.size(28.dp).background(t.bg.copy(alpha = 0.7f), RoundedCornerShape(999.dp)),
            )
            failed -> Text("⚠", color = t.danger, fontSize = 20.sp)
        }
        Text(
            "✕",
            color = t.ink,
            fontSize = 12.sp,
            modifier = Modifier
                .align(Alignment.TopEnd)
                .padding(2.dp)
                .background(t.surface, RoundedCornerShape(999.dp))
                .clickable { attach.onRemove(a.key) }
                .padding(horizontal = 6.dp, vertical = 2.dp),
        )
    }
}

/** Corner rounding of the message bar. */
private val BAR_RADIUS = 24.dp

/** Size of a picked file's preview above the message bar, and the pixels decoded for it. */
private val PREVIEW_DP = 64.dp
private const val PREVIEW_PX = 256

/** Any kind of file, for the file picker. */
private const val ANY_FILE = "*/*"

/** A permission or plan card: the buttons until it's settled (here or on another device). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Waiting(
    item: ChatItem.Waiting,
    connected: Boolean,
    onAnswer: (String, String) -> Unit,
    onDecide: (String, Boolean, Boolean) -> Unit,
) {
    val t = LocalTokens.current
    // Sent and waiting for the engine to confirm: no second tap.
    var sent by remember(item.key) { mutableStateOf(false) }
    // If the answer never lands (the link dropped), offer the buttons again.
    LaunchedEffect(sent) {
        if (sent) {
            kotlinx.coroutines.delay(RESEND_AFTER_MS)
            sent = false
        }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .border(1.dp, t.accent.copy(alpha = 0.5f), RoundedCornerShape(12.dp))
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(if (item.plan) "Plan waiting for approval" else "Asks permission", color = t.accent, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        val settled = item.settled
        // Once answered, a long command shrinks to a few lines; tap to see it all again.
        var expanded by remember(item.key) { mutableStateOf(false) }
        val folded = settled != null && !expanded
        Box(Modifier.clickable(enabled = settled != null) { expanded = !expanded }.animateContentSize()) {
            if (item.plan && !folded) {
                Markdown(item.what)
            } else {
                Text(
                    item.what,
                    color = t.ink,
                    fontSize = 14.sp,
                    maxLines = if (folded) SETTLED_LINES else Int.MAX_VALUE,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        when {
            settled != null -> Text(settled, color = t.inkMuted, fontSize = 12.sp)
            sent -> Text("Sending…", color = t.inkMuted, fontSize = 12.sp)
            else -> FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (item.plan) {
                    Button(onClick = { sent = true; onDecide(item.key, true, false) }, enabled = connected) { Text("Approve") }
                    OutlinedButton(onClick = { sent = true; onDecide(item.key, true, true) }, enabled = connected) { Text("Approve + auto edits") }
                    TextButton(onClick = { sent = true; onDecide(item.key, false, false) }, enabled = connected) { Text("Not yet") }
                } else {
                    if ("yes" in item.choices) {
                        Button(onClick = { sent = true; onAnswer(item.key, "yes") }, enabled = connected) { Text("Yes") }
                    }
                    if ("always" in item.choices) {
                        OutlinedButton(onClick = { sent = true; onAnswer(item.key, "always") }, enabled = connected) { Text("Always (this session)") }
                    }
                    if ("no" in item.choices) {
                        TextButton(onClick = { sent = true; onAnswer(item.key, "no") }, enabled = connected) { Text("No") }
                    }
                }
            }
        }
    }
}

/** How long a sent answer waits for the engine before the buttons come back. */
private const val RESEND_AFTER_MS = 10_000L

/** Lines of an answered card left showing (tap for the rest). */
private const val SETTLED_LINES = 3

/**
 * What a conversation needs to show and open files: fetching, its session, opening a file, showing
 * one in the session's folder ([showIn], a path there), and sending a copy to another chat.
 */
class FileActions(
    val downloads: Downloads,
    val session: String,
    val open: (FileSource) -> Unit,
    val showIn: ((String) -> Unit)? = null,
    val forward: ((java.io.File, String) -> Unit)? = null,
)

/** Files on a message or step: pictures shown, everything else as a chip. Tap opens, hold for more. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Attached(refs: List<FileRef>, files: FileActions) {
    if (refs.isEmpty()) return
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (ref in refs) {
            val source = FileSource.Upload(ref.file, ref.name)
            if (ref.kind == "image" || FileKinds.kind(ref.name) == FileKind.Image) {
                Picture(files, source)
            } else {
                FileChip("📎 ${ref.name}", source, files)
            }
        }
    }
}

/** A file in the session's folder that a step made or changed, as a card: tap opens, hold for more. */
@Composable
private fun ChangedFile(diff: Diff, files: FileActions) {
    if (diff.kind == "delete") return
    val source = FileSource.Folder(files.session, diff.path)
    if (FileKinds.kind(diff.path) == FileKind.Image) {
        Picture(files, source)
        return
    }
    val what = if (diff.kind == "create") "New file" else "Changed"
    FileChip("📄 ${source.name} · $what", source, files)
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileChip(label: String, source: FileSource, files: FileActions) {
    val t = LocalTokens.current
    var menu by remember(source) { mutableStateOf(false) }
    Box {
        Text(
            label,
            color = t.accent,
            fontSize = 13.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .background(t.surface, RoundedCornerShape(999.dp))
                .combinedClickable(onClick = { files.open(source) }, onLongClick = { menu = true })
                .padding(horizontal = 12.dp, vertical = 6.dp),
        )
        FileMenu(source, files, expanded = menu, onDismiss = { menu = false })
    }
}

/** A picture in the conversation, fetched once and shown small; tap for the full screen, hold for more. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Picture(files: FileActions, source: FileSource) {
    var menu by remember(source) { mutableStateOf(false) }
    Box {
        PictureBox(files.downloads, source, onOpen = { files.open(source) }, onHold = { menu = true })
        FileMenu(source, files, expanded = menu, onDismiss = { menu = false })
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PictureBox(downloads: Downloads, source: FileSource, onOpen: () -> Unit, onHold: () -> Unit) {
    val t = LocalTokens.current
    var image by remember(source) { mutableStateOf<ImageBitmap?>(null) }
    var failed by remember(source) { mutableStateOf(false) }
    LaunchedEffect(source) {
        try {
            val cached = downloads.target(source)
            val file = if (cached.exists()) cached else downloads.fetch(source)
            image = withContext(Dispatchers.IO) { decode(file, PICTURE_THUMB_PX) }
            failed = image == null
        } catch (e: Exception) {
            failed = true
        }
    }
    val shown = image
    Box(
        Modifier
            .widthIn(max = 280.dp)
            .heightIn(min = 60.dp, max = 280.dp)
            .background(t.surface, RoundedCornerShape(10.dp))
            .combinedClickable(onClick = onOpen, onLongClick = onHold),
        contentAlignment = Alignment.Center,
    ) {
        when {
            shown != null -> Image(shown, contentDescription = source.name, contentScale = ContentScale.Fit)
            failed -> Text("▣ ${source.name}", color = t.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(12.dp))
            else -> Text("Loading picture…", color = t.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(12.dp))
        }
    }
}

/** Longest side of a picture shown in the conversation, in pixels. */
private const val PICTURE_THUMB_PX = 1024
