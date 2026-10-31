package app.thrall.ui

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.chat.SessionLayout
import app.thrall.data.UpdateState
import androidx.compose.ui.platform.LocalContext
import app.thrall.protocol.SessionInfo

/** What can be done to a chat in the list (hold it down). */
class SessionActions(
    val onRename: (String, String) -> Unit,
    val onPin: (String, Boolean) -> Unit,
    val onArchive: (String, Boolean) -> Unit,
    val onDelete: (String) -> Unit,
    val onShowArchived: (Boolean) -> Unit,
    /** Put a chat in a group (null takes it out). */
    val onMove: (String, String?) -> Unit = { _, _ -> },
    /** A new group, with the chat moved into it. */
    val onNewGroup: (String, String?) -> Unit = { _, _ -> },
    val onRenameGroup: (String, String) -> Unit = { _, _ -> },
    /** Delete a group; its chats stay. */
    val onDeleteGroup: (String) -> Unit = {},
)

/** The ⋮ menu's phone settings: app lock, and the app's own updates. */
class PhoneMenu(
    val lockOn: Boolean,
    val onLock: (Boolean) -> Unit,
    val stayOn: Boolean,
    val onStay: (Boolean) -> Unit,
    val update: UpdateState,
    val onUpdate: () -> Unit,
    val onUnpair: () -> Unit,
    /** The other screens: groups, Home and check-ins, memory, you and your key, settings. */
    val onOpen: (MoreScreen) -> Unit = {},
    /** Whose phone this is, once the engine has said. */
    val signedInAs: String? = null,
    /** The phone is the owner's: the screens that change Thrall for everyone are shown. */
    val owner: Boolean = true,
)

/** Screens the ⋮ menu opens; [ownerOnly] ones change Thrall for everyone. */
enum class MoreScreen(val label: String, val ownerOnly: Boolean = false) {
    Groups("Groups…"),
    Home("Home and check-ins…", ownerOnly = true),
    Memory("What Thrall remembers…"),
    Account("You and your key…"),
    Settings("Settings…"),
}

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun SessionsScreen(
    shown: SessionsModel.Shown,
    onOpen: (SessionInfo) -> Unit,
    onNew: () -> Unit,
    onRefresh: () -> Unit,
    actions: SessionActions,
    menu: PhoneMenu,
) {
    val t = LocalTokens.current
    var renaming by remember { mutableStateOf<SessionInfo?>(null) }
    var deleting by remember { mutableStateOf<SessionInfo?>(null) }
    var moving by remember { mutableStateOf<SessionInfo?>(null) }
    // Sections folded on this phone, kept across opening a chat and restarting the app.
    val context = LocalContext.current
    val foldStore = remember { FoldedSections(context) }
    var folded by remember { mutableStateOf(foldStore.keys) }
    var makingGroup by remember { mutableStateOf(false) }
    var renamingGroup by remember { mutableStateOf<app.thrall.protocol.GroupInfo?>(null) }
    var deletingGroup by remember { mutableStateOf<app.thrall.protocol.GroupInfo?>(null) }
    var unpairing by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxSize()) {
        PullToRefreshBox(isRefreshing = shown.loading, onRefresh = onRefresh, modifier = Modifier.fillMaxSize()) {
            LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(0.dp)) {
                item {
                    Row(Modifier.fillMaxWidth().padding(16.dp, 16.dp, 4.dp, 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("Thrall", fontSize = 24.sp, fontWeight = FontWeight.SemiBold, color = t.ink)
                            menu.signedInAs?.let { Text("Signed in as $it", fontSize = 12.sp, color = t.inkMuted) }
                        }
                        Menu(shown.archived, menu, actions, onUnpair = { unpairing = true })
                    }
                }
                item { UpdateBanner(menu.update, menu.onUpdate) }
                if (!shown.archived) {
                    shown.home?.let { home ->
                        item {
                            Column(
                                Modifier
                                    .padding(horizontal = 16.dp, vertical = 8.dp)
                                    .fillMaxWidth()
                                    .background(t.accentSoft, RoundedCornerShape(14.dp))
                                    .clickable { onOpen(home) }
                                    .padding(16.dp),
                            ) {
                                Text("Home", fontWeight = FontWeight.SemiBold, color = t.ink, fontSize = 17.sp)
                                Text("Your own conversation with Thrall", color = t.inkMuted, fontSize = 13.sp)
                            }
                        }
                    }
                }
                shown.problem?.let { item { Text(it, color = t.danger, fontSize = 13.sp, modifier = Modifier.padding(16.dp)) } }
                if (shown.archived) {
                    item {
                        Row(Modifier.fillMaxWidth().padding(start = 16.dp, top = 16.dp, bottom = 4.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text("ARCHIVED", color = t.inkMuted, fontSize = 11.sp, letterSpacing = 1.sp, modifier = Modifier.weight(1f))
                            TextButton(onClick = { actions.onShowArchived(false) }) { Text("Back to chats") }
                        }
                    }
                }
                // As the web app shows them: Pinned, the person's groups, then by folder. Tap a
                // heading to fold it.
                val sections = if (shown.archived) {
                    listOf(SessionLayout.Section("archived", "", shown.sessions))
                } else {
                    SessionLayout.arrange(shown.sessions, shown.groups)
                }
                var part: SessionLayout.Part? = null
                for (section in sections) {
                    // Groups and folders look alike: each part of the list gets a labelled divider.
                    if (!shown.archived && section.part != part) {
                        // No groups yet: still show where they'd go, and how to make one.
                        if (section.part == SessionLayout.Part.Folders && shown.groups.isEmpty()) {
                            item(key = "part:no-groups") {
                                Column {
                                    PartLabel("Groups", onNew = { makingGroup = true })
                                    Text("Make a group to keep chats together.", color = t.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 16.dp))
                                }
                            }
                        }
                        part = section.part
                        if (section.part != SessionLayout.Part.Pinned) {
                            val groupsPart = section.part == SessionLayout.Part.Groups
                            item(key = "part:${section.part}") {
                                PartLabel(if (groupsPart) "Groups" else "Folders", onNew = if (groupsPart) ({ makingGroup = true }) else null)
                            }
                        }
                    }
                    val isFolded = section.key in folded
                    if (section.label.isNotEmpty()) {
                        item(key = "head:${section.key}") {
                            // Tap folds; hold a group's heading to rename or delete the group.
                            var groupMenu by remember { mutableStateOf(false) }
                            Box {
                            Row(
                                Modifier
                                    .fillMaxWidth()
                                    .combinedClickable(
                                        onClick = {
                                            folded = if (isFolded) folded - section.key else folded + section.key
                                            foldStore.keys = folded
                                        },
                                        onLongClick = section.group?.let { { groupMenu = true } },
                                    )
                                    .padding(start = 16.dp, end = 16.dp, top = 18.dp, bottom = 6.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text(if (isFolded) "▸ " else "▾ ", color = t.inkMuted, fontSize = 12.sp)
                                Column(Modifier.weight(1f)) {
                                    Text(
                                        section.label.uppercase(),
                                        color = t.inkMuted,
                                        fontSize = 11.sp,
                                        letterSpacing = 1.sp,
                                        fontWeight = FontWeight.SemiBold,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    section.folder?.let { Text(it, color = t.inkMuted, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                                }
                                Text("${section.sessions.size}", color = t.inkMuted, fontSize = 11.sp)
                            }
                            section.group?.let { g ->
                                DropdownMenu(expanded = groupMenu, onDismissRequest = { groupMenu = false }) {
                                    DropdownMenuItem(text = { Text("Rename group") }, onClick = { groupMenu = false; renamingGroup = g })
                                    DropdownMenuItem(text = { Text("Delete group…", color = t.danger) }, onClick = { groupMenu = false; deletingGroup = g })
                                }
                            }
                            }
                        }
                    }
                    if (isFolded) continue
                    if (section.group != null && section.sessions.isEmpty()) {
                        item(key = "empty:${section.key}") {
                            Text("Hold a chat and pick Move to group… to put it here.", color = t.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                        }
                    }
                    items(section.sessions, key = { "${section.key}/${it.session}" }) { s ->
                        SessionRow(
                            s,
                            archived = shown.archived,
                            showFolder = section.showFolder,
                            onOpen = onOpen,
                            actions = actions,
                            onRename = { renaming = s },
                            onDelete = { deleting = s },
                            onMove = { moving = s },
                        )
                        HorizontalDivider(color = t.line, modifier = Modifier.padding(start = 16.dp))
                    }
                }
                if (!shown.loading && shown.sessions.isEmpty()) {
                    item {
                        Text(
                            if (shown.archived) "No archived chats." else "No sessions yet.",
                            color = t.inkMuted,
                            modifier = Modifier.padding(16.dp),
                        )
                    }
                }
                if (!shown.archived && shown.sessions.isNotEmpty()) {
                    item {
                        Text(
                            "Hold a chat down to rename, pin, group, archive or delete it.",
                            color = t.inkMuted,
                            fontSize = 12.sp,
                            modifier = Modifier.padding(16.dp, 12.dp, 16.dp, 88.dp),
                        )
                    }
                }
            }
        }
        if (!shown.archived) {
            ExtendedFloatingActionButton(
                onClick = onNew,
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
                text = { Text("New chat") },
                icon = {},
            )
        }
    }
    renaming?.let { s -> RenameDialog(s, onDone = { title -> if (title != null) actions.onRename(s.session, title); renaming = null }) }
    moving?.let { s -> MoveDialog(s, shown.groups, actions, onDone = { moving = null }) }
    renamingGroup?.let { g ->
        var name by remember(g) { mutableStateOf(g.name) }
        AlertDialog(
            onDismissRequest = { renamingGroup = null },
            title = { Text("Rename group") },
            text = { OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true) },
            confirmButton = { TextButton(onClick = { actions.onRenameGroup(g.group, name); renamingGroup = null }, enabled = name.isNotBlank()) { Text("Rename") } },
            dismissButton = { TextButton(onClick = { renamingGroup = null }) { Text("Cancel") } },
        )
    }
    deletingGroup?.let { g ->
        AlertDialog(
            onDismissRequest = { deletingGroup = null },
            title = { Text("Delete the “${g.name}” group?") },
            text = { Text("Only the group goes. Its chats stay, back under their folders.") },
            confirmButton = { TextButton(onClick = { actions.onDeleteGroup(g.group); deletingGroup = null }) { Text("Delete group", color = t.danger) } },
            dismissButton = { TextButton(onClick = { deletingGroup = null }) { Text("Cancel") } },
        )
    }
    if (makingGroup) {
        var name by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { makingGroup = false },
            title = { Text("New group") },
            text = { OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true, placeholder = { Text("Its name") }) },
            confirmButton = { TextButton(onClick = { actions.onNewGroup(name, null); makingGroup = false }, enabled = name.isNotBlank()) { Text("Make it") } },
            dismissButton = { TextButton(onClick = { makingGroup = false }) { Text("Cancel") } },
        )
    }
    deleting?.let { s ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete this chat?") },
            text = { Text("“${s.title.ifBlank { "New session" }}” and everything in it will be gone for good. Files it made on the computer stay.") },
            confirmButton = { TextButton(onClick = { actions.onDelete(s.session); deleting = null }) { Text("Delete", color = t.danger) } },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Keep it") } },
        )
    }
    if (unpairing) {
        AlertDialog(
            onDismissRequest = { unpairing = false },
            title = { Text("Unpair this phone?") },
            text = { Text("It stops reaching Thrall. To use it again you'll need a new pairing code from the computer.") },
            confirmButton = { TextButton(onClick = { unpairing = false; menu.onUnpair() }) { Text("Unpair", color = t.danger) } },
            dismissButton = { TextButton(onClick = { unpairing = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Menu(archived: Boolean, menu: PhoneMenu, actions: SessionActions, onUnpair: () -> Unit) {
    val t = LocalTokens.current
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) { Text("⋮", fontSize = 22.sp, color = t.inkMuted) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(if (archived) "Back to chats" else "Archived chats") },
                onClick = { open = false; actions.onShowArchived(!archived) },
            )
            DropdownMenuItem(
                text = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("App lock")
                            Text("Fingerprint, face or screen lock", fontSize = 12.sp, color = t.inkMuted)
                        }
                        Switch(checked = menu.lockOn, onCheckedChange = null)
                    }
                },
                onClick = { open = false; menu.onLock(!menu.lockOn) },
            )
            DropdownMenuItem(
                text = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("Stay connected")
                            Text("Notifications while the app is closed", fontSize = 12.sp, color = t.inkMuted)
                        }
                        Switch(checked = menu.stayOn, onCheckedChange = null)
                    }
                },
                onClick = { open = false; menu.onStay(!menu.stayOn) },
            )
            for (screen in MoreScreen.entries.filter { menu.owner || !it.ownerOnly }) {
                DropdownMenuItem(text = { Text(screen.label) }, onClick = { open = false; menu.onOpen(screen) })
            }
            DropdownMenuItem(text = { Text("Unpair this phone", color = t.danger) }, onClick = { open = false; onUnpair() })
        }
    }
}

/** The app's own update, offered by the computer. */
@Composable
private fun UpdateBanner(update: UpdateState, onUpdate: () -> Unit) {
    val t = LocalTokens.current
    val (text, action) = when (update) {
        UpdateState.None -> return
        is UpdateState.Available -> "Thrall ${update.release.versionName} is ready to install." to "Install"
        is UpdateState.NeedsPermission ->
            "Android needs your OK for Thrall to install updates (only once). Tap Continue, switch it on, come back, then tap Continue again." to "Continue"
        is UpdateState.Downloading -> {
            val pct = if (update.release.size > 0) update.got * 100 / update.release.size else 0
            "Downloading ${update.release.versionName}… $pct%" to null
        }
        is UpdateState.Installing -> "Installing ${update.release.versionName}: confirm on the screen Android shows." to null
        is UpdateState.Failed -> "The update didn't install: ${update.reason}" to "Try again"
    }
    val notes = (update as? UpdateState.Available)?.release?.notes?.takeIf { it.isNotBlank() }
    Column(
        Modifier
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .fillMaxWidth()
            .background(t.surface, RoundedCornerShape(12.dp))
            .padding(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(text, color = t.ink, fontSize = 14.sp, modifier = Modifier.weight(1f))
            action?.let { TextButton(onClick = onUpdate) { Text(it) } }
        }
        notes?.let { Text("What's new: $it", color = t.inkMuted, fontSize = 12.sp) }
    }
}

/** "Groups" or "Folders": a divider line and a quieter label, with "+ New" for groups. */
@Composable
private fun PartLabel(text: String, onNew: (() -> Unit)?) {
    val t = LocalTokens.current
    Column(Modifier.fillMaxWidth().padding(top = 14.dp)) {
        HorizontalDivider(color = t.line, modifier = Modifier.padding(horizontal = 16.dp))
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, color = t.inkMuted, fontSize = 13.sp, modifier = Modifier.weight(1f))
            if (onNew != null) TextButton(onClick = onNew) { Text("+ New", color = t.accent, fontSize = 13.sp) }
        }
    }
}

/** Put a chat in a group, take it out, or make a new group for it. */
@Composable
private fun MoveDialog(s: SessionInfo, groups: List<app.thrall.protocol.GroupInfo>, actions: SessionActions, onDone: () -> Unit) {
    val t = LocalTokens.current
    var newName by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDone,
        title = { Text("Move to group") },
        text = {
            Column {
                val pick: (String?) -> Unit = { g -> actions.onMove(s.session, g); onDone() }
                Text(
                    (if (s.group == null) "✓ " else "") + "No group",
                    color = t.ink,
                    modifier = Modifier.fillMaxWidth().clickable { pick(null) }.padding(vertical = 10.dp),
                )
                for (g in groups) {
                    Text(
                        (if (s.group == g.group) "✓ " else "") + g.name,
                        color = t.ink,
                        modifier = Modifier.fillMaxWidth().clickable { pick(g.group) }.padding(vertical = 10.dp),
                    )
                }
                OutlinedTextField(
                    value = newName,
                    onValueChange = { newName = it },
                    placeholder = { Text("Or a new group") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { actions.onNewGroup(newName, s.session); onDone() }, enabled = newName.isNotBlank()) { Text("Make it and move") }
        },
        dismissButton = { TextButton(onClick = onDone) { Text("Cancel") } },
    )
}

@Composable
private fun RenameDialog(s: SessionInfo, onDone: (String?) -> Unit) {
    var title by remember { mutableStateOf(s.title) }
    AlertDialog(
        onDismissRequest = { onDone(null) },
        title = { Text("Rename chat") },
        text = { OutlinedTextField(value = title, onValueChange = { title = it }, singleLine = true) },
        confirmButton = { TextButton(onClick = { onDone(title) }, enabled = title.isNotBlank()) { Text("Rename") } },
        dismissButton = { TextButton(onClick = { onDone(null) }) { Text("Cancel") } },
    )
}

@Composable
private fun SessionRow(
    s: SessionInfo,
    archived: Boolean,
    showFolder: Boolean,
    onOpen: (SessionInfo) -> Unit,
    actions: SessionActions,
    onRename: () -> Unit,
    onDelete: () -> Unit,
    onMove: () -> Unit,
) {
    val t = LocalTokens.current
    var menu by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .fillMaxWidth()
                .combinedClickable(onClick = { onOpen(s) }, onLongClick = { menu = true })
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    (if (s.pinned) "📌 " else "") + s.title.ifBlank { "New session" },
                    color = t.ink,
                    fontSize = 15.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val folder = s.workspace?.takeIf { showFolder }?.let { SessionLayout.folderName(it) }
                val ago = DateUtils.getRelativeTimeSpanString(s.updated, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS)
                Text(listOfNotNull(folder, ago.toString()).joinToString(" · "), color = t.inkMuted, fontSize = 12.sp, maxLines = 1)
            }
            when (s.state) {
                "working" -> Badge("Working", t.warn)
                "waiting_for_you" -> Badge("Needs you", t.accent)
            }
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(text = { Text("Rename") }, onClick = { menu = false; onRename() })
            if (!archived) {
                DropdownMenuItem(text = { Text(if (s.pinned) "Unpin" else "Pin to top") }, onClick = { menu = false; actions.onPin(s.session, !s.pinned) })
                DropdownMenuItem(text = { Text("Move to group…") }, onClick = { menu = false; onMove() })
            }
            DropdownMenuItem(
                text = { Text(if (archived) "Bring back" else "Archive") },
                onClick = { menu = false; actions.onArchive(s.session, !archived) },
            )
            DropdownMenuItem(text = { Text("Delete…", color = t.danger) }, onClick = { menu = false; onDelete() })
        }
    }
}

@Composable
fun Badge(text: String, color: androidx.compose.ui.graphics.Color) {
    Text(
        text,
        color = color,
        fontSize = 11.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier
            .background(color.copy(alpha = 0.14f), RoundedCornerShape(999.dp))
            .padding(horizontal = 8.dp, vertical = 2.dp),
    )
}
