package app.thrall.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.data.Connection
import app.thrall.data.RequestFailed
import app.thrall.protocol.Reply
import app.thrall.protocol.Request

/**
 * Pick the folder on the computer a chat works in: folders other sessions use, Thrall's own
 * (`Documents\Thrall`), or a typed path. Used for a new chat and for moving one.
 */
@Composable
fun FolderPicker(
    connection: Connection,
    title: String,
    current: String?,
    problem: String?,
    onPick: (String) -> Unit,
    onBack: () -> Unit,
) {
    val t = LocalTokens.current
    var places by remember { mutableStateOf<Reply.FolderPlaces?>(null) }
    var loadProblem by remember { mutableStateOf<String?>(null) }
    var typed by remember { mutableStateOf("") }
    LaunchedEffect(Unit) {
        try {
            places = connection.request(Request.FolderPlaces) as? Reply.FolderPlaces
        } catch (e: RequestFailed) {
            loadProblem = if (e.code == "unsupported") "Update Thrall on the computer to pick folders from the phone." else e.message
        } catch (e: Exception) {
            loadProblem = e.message
        }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(title = title, subtitle = current?.let { "Now: $it" }, onBack = onBack)
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            (problem ?: loadProblem)?.let { Text(it, color = t.danger, fontSize = 13.sp) }
            val p = places
            if (p == null && loadProblem == null) Text("Loading folders…", color = t.inkMuted, fontSize = 13.sp)
            p?.let { NewFolder(connection, it, onMade = onPick) }
            p?.default?.let { Place(it, "Thrall's own folder (made if it's not there yet)", it == current) { onPick(it) } }
            if (!p?.recent.isNullOrEmpty()) {
                Text("USED BY YOUR SESSIONS", color = t.inkMuted, fontSize = 11.sp, letterSpacing = 1.sp, modifier = Modifier.padding(top = 8.dp))
                for (folder in p!!.recent.filter { it != p.default }) {
                    Place(folder, null, folder == current) { onPick(folder) }
                }
            }
            Text("OR TYPE A PATH ON THE COMPUTER", color = t.inkMuted, fontSize = 11.sp, letterSpacing = 1.sp, modifier = Modifier.padding(top = 8.dp))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = typed,
                    onValueChange = { typed = it },
                    placeholder = { Text("C:\\Users\\you\\Projects\\…") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        autoCorrectEnabled = false,
                        keyboardType = KeyboardType.Uri,
                    ),
                    modifier = Modifier.weight(1f),
                )
                Button(onClick = { onPick(typed.trim()) }, enabled = typed.isNotBlank()) { Text("Use") }
            }
        }
    }
}

/**
 * "＋ New folder": a name, and where it goes (Thrall's new-folder place first, then the others
 * offered). Made on the computer, then picked.
 */
@Composable
private fun NewFolder(connection: Connection, places: Reply.FolderPlaces, onMade: (String) -> Unit) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    val parents = remember(places) { listOfNotNull(places.newIn, places.default).plus(places.recent).distinct() }
    var parent by remember(places) { mutableStateOf(parents.firstOrNull()) }
    var busy by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }
    if (!open) {
        Text(
            "＋ New folder",
            color = t.accent,
            fontWeight = FontWeight.SemiBold,
            fontSize = 15.sp,
            modifier = Modifier
                .fillMaxWidth()
                .background(t.surface, RoundedCornerShape(10.dp))
                .clickable { open = true }
                .padding(horizontal = 12.dp, vertical = 12.dp),
        )
        return
    }
    Column(
        Modifier.fillMaxWidth().background(t.surface, RoundedCornerShape(10.dp)).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("New folder", color = t.ink, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
        OutlinedTextField(
            value = name,
            onValueChange = { name = it; problem = null },
            placeholder = { Text("Its name") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Text("IN", color = t.inkMuted, fontSize = 11.sp, letterSpacing = 1.sp)
        for (folder in parents) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(if (folder == parent) t.accentSoft else t.bg, RoundedCornerShape(8.dp))
                    .clickable { parent = folder }
                    .padding(horizontal = 10.dp, vertical = 8.dp),
            ) {
                Column {
                    Text(folderName(folder), color = t.ink, fontSize = 14.sp)
                    Text(folder, color = t.inkMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        problem?.let { Text(it, color = t.danger, fontSize = 13.sp) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                enabled = name.isNotBlank() && parent != null && !busy,
                onClick = {
                    val into = parent ?: return@Button
                    busy = true
                    scope.launch {
                        try {
                            val made = connection.request(Request.FolderCreate(into, name.trim())) as? Reply.FolderCreated
                            if (made != null) onMade(made.path)
                        } catch (e: RequestFailed) {
                            problem = if (e.code == "unsupported") "Update Thrall on the computer to make folders from the phone." else e.message
                        } catch (e: Exception) {
                            problem = e.message
                        }
                        busy = false
                    }
                },
            ) { Text(if (busy) "Making…" else "Make it and use it") }
            androidx.compose.material3.TextButton(onClick = { open = false; problem = null }) { Text("Cancel") }
        }
    }
}

@Composable
private fun Place(folder: String, note: String?, isCurrent: Boolean, onPick: () -> Unit) {
    val t = LocalTokens.current
    Column(
        Modifier
            .fillMaxWidth()
            .background(if (isCurrent) t.accentSoft else t.surface, RoundedCornerShape(10.dp))
            .clickable(onClick = onPick)
            .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        Text(folderName(folder), color = t.ink, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
        Text(folder, color = t.inkMuted, fontSize = 12.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        note?.let { Text(it, color = t.inkMuted, fontSize = 11.sp) }
    }
}

/** The last part of a folder path (`C:\Users\me\Thrall` → `Thrall`). */
fun folderName(path: String): String =
    path.trimEnd('\\', '/').substringAfterLast('\\').substringAfterLast('/').ifEmpty { path }
