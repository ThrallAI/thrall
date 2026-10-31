package app.thrall.ui

import android.text.format.DateUtils
import androidx.compose.foundation.background
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.data.Connection
import app.thrall.data.RequestFailed
import app.thrall.data.UpdateState
import app.thrall.protocol.FactInfo
import app.thrall.protocol.GroupInfo
import app.thrall.protocol.PersonInfo
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import app.thrall.protocol.SkillInfo
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.time.ZonedDateTime
import java.time.temporal.ChronoUnit
import java.util.Locale

/** A failed request, in words for the screen. */
private fun why(e: Exception): String =
    if (e is RequestFailed && e.code == "unsupported") "Update Thrall on the computer for this." else e.message ?: "That didn't work."

@Composable
private fun Section(text: String) {
    val t = LocalTokens.current
    Text(text.uppercase(), color = t.inkMuted, fontSize = 11.sp, letterSpacing = 1.sp, modifier = Modifier.padding(top = 12.dp))
}

/** A question before something can't be undone; [onYes] runs only on yes. */
@Composable
private fun Confirm(title: String, text: String, yes: String, onYes: () -> Unit, onDone: () -> Unit) {
    val t = LocalTokens.current
    AlertDialog(
        onDismissRequest = onDone,
        title = { Text(title) },
        text = { Text(text) },
        confirmButton = { TextButton(onClick = { onYes(); onDone() }) { Text(yes, color = t.danger) } },
        dismissButton = { TextButton(onClick = onDone) { Text("Cancel") } },
    )
}

/** The person's groups: make, rename, reorder, delete (their chats stay, ungrouped). */
@Composable
fun GroupsScreen(shown: SessionsModel.Shown, model: SessionsModel, onBack: () -> Unit) {
    val t = LocalTokens.current
    var name by remember { mutableStateOf("") }
    var renaming by remember { mutableStateOf<GroupInfo?>(null) }
    var deleting by remember { mutableStateOf<GroupInfo?>(null) }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(title = "Groups", subtitle = "Chats can sit in a group; hold a chat to move it.", onBack = onBack)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            shown.problem?.let { Text(it, color = t.danger, fontSize = 13.sp) }
            if (shown.groups.isEmpty()) Text("No groups yet.", color = t.inkMuted)
            shown.groups.forEachIndexed { i, g ->
                val count = shown.sessions.count { it.group == g.group }
                Row(
                    Modifier.fillMaxWidth().background(t.surface, RoundedCornerShape(10.dp)).padding(start = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(g.name, color = t.ink, fontWeight = FontWeight.SemiBold)
                        Text(if (count == 1) "1 chat" else "$count chats", color = t.inkMuted, fontSize = 12.sp)
                    }
                    TextButton(onClick = { model.shiftGroup(g.group, -1) }, enabled = i > 0) { Text("↑") }
                    TextButton(onClick = { model.shiftGroup(g.group, 1) }, enabled = i < shown.groups.lastIndex) { Text("↓") }
                    TextButton(onClick = { renaming = g }) { Text("Rename") }
                    TextButton(onClick = { deleting = g }) { Text("✕", color = t.danger) }
                }
            }
            Section("New group")
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true, placeholder = { Text("Its name") }, modifier = Modifier.weight(1f))
                Button(onClick = { model.newGroup(name, null); name = "" }, enabled = name.isNotBlank()) { Text("Make") }
            }
        }
    }
    renaming?.let { g ->
        var newName by remember(g) { mutableStateOf(g.name) }
        AlertDialog(
            onDismissRequest = { renaming = null },
            title = { Text("Rename group") },
            text = { OutlinedTextField(value = newName, onValueChange = { newName = it }, singleLine = true) },
            confirmButton = { TextButton(onClick = { model.renameGroup(g.group, newName); renaming = null }, enabled = newName.isNotBlank()) { Text("Rename") } },
            dismissButton = { TextButton(onClick = { renaming = null }) { Text("Cancel") } },
        )
    }
    deleting?.let { g ->
        Confirm("Delete “${g.name}”?", "The group goes; its chats stay, out of any group.", "Delete", onYes = { model.deleteGroup(g.group) }, onDone = { deleting = null })
    }
}

/** Home and check-ins: the checklist Thrall goes through, checking in now, and clearing Home. */
@Composable
fun HomeScreen(connection: Connection, onBack: () -> Unit) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var shown by remember { mutableStateOf<Reply.HeartbeatShown?>(null) }
    var checklist by remember { mutableStateOf("") }
    var note by remember { mutableStateOf<String?>(null) }
    var clearing by remember { mutableStateOf(false) }
    val run: (suspend () -> Unit) -> Unit = { work ->
        scope.launch {
            try {
                work()
            } catch (e: Exception) {
                note = why(e)
            }
        }
    }
    LaunchedEffect(Unit) {
        run {
            shown = connection.request(Request.HeartbeatGet) as? Reply.HeartbeatShown
            checklist = shown?.checklist.orEmpty()
        }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(title = "Home and check-ins", subtitle = null, onBack = onBack)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            note?.let { Text(it, color = t.inkMuted, fontSize = 13.sp) }
            Section("Check-ins")
            Text("Now and then Thrall goes through this list and speaks up in Home when something needs you.", color = t.inkMuted, fontSize = 13.sp)
            val next = shown?.nextRun
            Text(
                if (next != null) "Next check-in: " + DateUtils.getRelativeTimeSpanString(next, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS)
                else "Check-ins are off (switch them on in Settings), or there's no list yet.",
                color = t.ink,
                fontSize = 13.sp,
            )
            OutlinedTextField(
                value = checklist,
                onValueChange = { checklist = it },
                placeholder = { Text("- Any chat waiting on me?\n- Anything new in my Downloads folder?") },
                minLines = 5,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = {
                        run {
                            shown = connection.request(Request.HeartbeatSet(checklist)) as? Reply.HeartbeatShown
                            note = "Checklist saved."
                        }
                    },
                    enabled = shown != null && checklist != shown?.checklist,
                ) { Text("Save") }
                OutlinedButton(
                    onClick = { run { connection.request(Request.HeartbeatRun); note = "Checking in now: see Home." } },
                    enabled = !shown?.checklist.isNullOrBlank(),
                ) { Text("Check now") }
            }
            Section("Home")
            Text("Deletes every Home page now and starts a fresh one. Facts Thrall saved stay.", color = t.inkMuted, fontSize = 13.sp)
            OutlinedButton(onClick = { clearing = true }) { Text("Clear Home…", color = t.danger) }
        }
    }
    if (clearing) {
        Confirm(
            "Clear Home?",
            "Every Home page is deleted for good.",
            "Clear",
            onYes = { run { connection.request(Request.HomeClear); note = "Home is cleared." } },
            onDone = { clearing = false },
        )
    }
}

/** What Thrall remembers about the person, and forgetting a fact. */
@Composable
fun MemoryScreen(connection: Connection, onBack: () -> Unit) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var facts by remember { mutableStateOf<List<FactInfo>?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    var forgetting by remember { mutableStateOf<FactInfo?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    LaunchedEffect(reload) {
        try {
            facts = (connection.request(Request.MemoryList) as? Reply.MemoryListed)?.facts.orEmpty()
        } catch (e: Exception) {
            problem = why(e)
        }
    }
    Column(Modifier.fillMaxSize()) {
        Header(title = "What Thrall remembers", subtitle = facts?.let { "${it.size} facts" }, onBack = onBack)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            problem?.let { Text(it, color = t.danger, fontSize = 13.sp) }
            val list = facts
            when {
                list == null && problem == null -> Text("Loading…", color = t.inkMuted)
                list != null && list.isEmpty() -> Text("Nothing yet.", color = t.inkMuted)
            }
            for (f in list.orEmpty()) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
                    Column(Modifier.weight(1f)) {
                        if (f.topic.isNotBlank()) Text(f.topic, color = t.accent, fontSize = 12.sp)
                        Text(f.content, color = t.ink, fontSize = 14.sp)
                        if (f.at > 0) Text(DateUtils.getRelativeTimeSpanString(f.at).toString(), color = t.inkMuted, fontSize = 11.sp)
                    }
                    TextButton(onClick = { forgetting = f }) { Text("Forget", color = t.danger) }
                }
                HorizontalDivider(color = t.line)
            }
        }
    }
    forgetting?.let { f ->
        Confirm(
            "Forget this?",
            "Thrall won't recall “${f.content.take(FACT_PREVIEW)}” any more.",
            "Forget",
            onYes = {
                scope.launch {
                    try {
                        connection.request(Request.MemoryForget(f.fact))
                        reload++
                    } catch (e: Exception) {
                        problem = why(e)
                    }
                }
            },
            onDone = { forgetting = null },
        )
    }
}

/**
 * Settings that make sense from the phone, as the computer has them, and the person's skills.
 * Only the [owner] can change them (they change Thrall for everyone); others see them.
 */
@Composable
fun SettingsScreen(connection: Connection, owner: Boolean, about: AboutActions, onBack: () -> Unit) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var shown by remember { mutableStateOf<Reply.SettingsShown?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    var removing by remember { mutableStateOf<SkillInfo?>(null) }
    val save: (Request) -> Unit = { request ->
        scope.launch {
            try {
                shown = connection.request(request) as? Reply.SettingsShown ?: shown
                problem = null
            } catch (e: Exception) {
                problem = why(e)
            }
        }
    }
    LaunchedEffect(Unit) {
        try {
            shown = connection.request(Request.SettingsGet) as? Reply.SettingsShown
        } catch (e: Exception) {
            problem = why(e)
        }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(title = "Settings", subtitle = shown?.model?.let { "Model now: $it" }, onBack = onBack)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            About(about)
            problem?.let { Text(it, color = t.danger, fontSize = 13.sp) }
            val s = shown
            if (s == null) {
                if (problem == null) Text("Loading…", color = t.inkMuted)
                return@Column
            }
            if (s.pending) Text("Some changes take effect when Thrall restarts on the computer.", color = t.warn, fontSize = 13.sp)
            Text(
                if (owner) "Changes here change Thrall on the computer, for every device."
                else "Only the person who runs this Thrall can change these.",
                color = t.inkMuted,
                fontSize = 13.sp,
            )
            for ((section, keys) in PHONE_SETTINGS) {
                Section(section)
                for ((key, label) in keys) {
                    val value = valueAt(s.config, key) ?: continue
                    SettingRow(key, label, value, enabled = owner) { save(Request.SettingsSet(key, it)) }
                }
            }
            val skills = s.skills
            if (skills.isNotEmpty()) {
                Section("Skills")
                for (skill in skills) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(skill.name, color = t.ink, fontSize = 14.sp)
                            if (skill.description.isNotBlank()) Text(skill.description, color = t.inkMuted, fontSize = 12.sp, maxLines = 2)
                        }
                        if (skill.personal) TextButton(onClick = { removing = skill }) { Text("Remove", color = t.danger) }
                        Switch(
                            checked = skill.enabled,
                            onCheckedChange = { save(Request.SettingsSet("skills.${skill.name}.enabled", JsonPrimitive(it))) },
                            enabled = owner,
                        )
                    }
                }
            }
        }
    }
    removing?.let { skill ->
        Confirm("Remove ${skill.name}?", "This skill of yours is deleted from the computer.", "Remove", onYes = { save(Request.SkillRemove(skill.name)) }, onDone = { removing = null })
    }
}

/**
 * What the About section needs: this app's version, the computer's (null while not connected),
 * a check for a newer app, and starting the update when there is one.
 */
class AboutActions(
    val appVersion: String,
    val engineVersion: String?,
    val check: suspend () -> UpdateState,
    val update: () -> Unit,
)

/** The app's and the computer's versions, and "Check for updates". */
@Composable
private fun About(about: AboutActions) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var checking by remember { mutableStateOf(false) }
    var answer by remember { mutableStateOf<String?>(null) }
    var newer by remember { mutableStateOf(false) }
    Section("About")
    Text("Thrall for Android ${about.appVersion}", color = t.ink, fontSize = 14.sp)
    Text(
        about.engineVersion?.let { "Thrall on the computer: $it" } ?: "Thrall on the computer: not connected",
        color = t.inkMuted,
        fontSize = 13.sp,
    )
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(
            enabled = !checking && about.engineVersion != null,
            onClick = {
                checking = true
                scope.launch {
                    answer = try {
                        when (val s = about.check()) {
                            is UpdateState.Available -> "Version ${s.release.versionName} is ready."
                            UpdateState.None -> "You have the newest version."
                            else -> null
                        }
                    } catch (e: Exception) {
                        why(e)
                    }
                    newer = answer?.startsWith("Version") == true
                    checking = false
                }
            },
        ) { Text(if (checking) "Checking…" else "Check for updates") }
        if (newer) Button(onClick = about.update) { Text("Update") }
    }
    answer?.let { Text(it, color = t.inkMuted, fontSize = 13.sp) }
}

/** One setting: a switch for yes/no, a box for a number or text (saved when you tap Save); only shown when not [enabled]. */
@Composable
private fun SettingRow(key: String, label: String, value: JsonPrimitive, enabled: Boolean, onSave: (JsonPrimitive) -> Unit) {
    val t = LocalTokens.current
    val flag = value.booleanOrNull
    if (flag != null && !value.isString) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(label, color = t.ink, fontSize = 14.sp, modifier = Modifier.weight(1f))
            Switch(checked = flag, onCheckedChange = { onSave(JsonPrimitive(it)) }, enabled = enabled)
        }
        return
    }
    val number = !value.isString && value.longOrNull != null
    var text by remember(key, value) { mutableStateOf(value.content) }
    Column {
        Text(label, color = t.ink, fontSize = 14.sp)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                enabled = enabled,
                keyboardOptions = KeyboardOptions(keyboardType = if (number) KeyboardType.Number else KeyboardType.Text),
                modifier = Modifier.weight(1f),
            )
            if (enabled) TextButton(
                onClick = { onSave(if (number) JsonPrimitive(text.trim().toLongOrNull() ?: 0) else JsonPrimitive(text.trim())) },
                enabled = text != value.content && (!number || text.trim().toLongOrNull() != null),
            ) { Text("Save") }
        }
    }
}

/** Who this phone is for: whose AI key their turns use (their own can be set here), their model and this month's spending. */
@Composable
fun AccountScreen(connection: Connection, onBack: () -> Unit) {
    val t = LocalTokens.current
    val scope = rememberCoroutineScope()
    var me by remember { mutableStateOf<PersonInfo?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var key by remember { mutableStateOf("") }
    var forgetting by remember { mutableStateOf(false) }
    var reload by remember { mutableIntStateOf(0) }
    LaunchedEffect(reload) {
        try {
            val listed = connection.request(Request.PeopleList(since = monthStart())) as? Reply.PeopleListed
            val id = connection.me.value?.id
            me = listed?.items?.firstOrNull { it.person == id } ?: listed?.items?.singleOrNull()
            problem = null
        } catch (e: Exception) {
            problem = why(e)
        }
    }
    val setKey: (String?) -> Unit = { value ->
        scope.launch {
            try {
                connection.request(Request.PeopleKey(value))
                key = ""
                note = if (value == null) "Your key is forgotten." else "Your key is kept on the computer. It's never shown again, here or there."
                reload++
            } catch (e: Exception) {
                problem = why(e)
            }
        }
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(title = "You and your key", subtitle = me?.name, onBack = onBack)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            problem?.let { Text(it, color = t.danger, fontSize = 13.sp) }
            note?.let { Text(it, color = t.inkMuted, fontSize = 13.sp) }
            val p = me
            if (p == null) {
                if (problem == null) Text("Loading…", color = t.inkMuted)
                return@Column
            }
            val owner = p.role == "owner"
            Text(if (owner) "You run this Thrall." else "You use this Thrall; someone else runs it on their computer.", color = t.ink, fontSize = 14.sp)
            Section("This month")
            Text(
                dollars(p.spent) + (p.monthlyLimit?.let { " of your ${dollars(it)} limit" } ?: ""),
                color = t.ink,
                fontSize = 14.sp,
            )
            Section("Model")
            Text(p.model?.let { "Your own: $it" } ?: "The one Thrall uses (see Settings).", color = t.ink, fontSize = 14.sp)
            Section("AI key")
            when {
                owner -> Text("Your turns use this Thrall's own key, set on the computer.", color = t.ink, fontSize = 14.sp)
                !p.ownKey -> Text(
                    "Your turns use the key of whoever runs this Thrall. They can switch you to your own on the computer's People page.",
                    color = t.ink,
                    fontSize = 14.sp,
                )
                else -> {
                    Text(
                        if (p.keySet) "Your turns use your own key, and it's set." else "Your turns use your own key, and it isn't set yet: Thrall can't answer you until it is.",
                        color = if (p.keySet) t.ink else t.warn,
                        fontSize = 14.sp,
                    )
                    Text("An OpenRouter key (openrouter.ai → Keys). Once saved it's never shown again.", color = t.inkMuted, fontSize = 13.sp)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = key,
                            onValueChange = { key = it },
                            singleLine = true,
                            placeholder = { Text(if (p.keySet) "A new key" else "sk-or-…") },
                            visualTransformation = PasswordVisualTransformation(),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                            modifier = Modifier.weight(1f),
                        )
                        Button(onClick = { setKey(key.trim()) }, enabled = key.isNotBlank()) { Text("Save") }
                    }
                    if (p.keySet) OutlinedButton(onClick = { forgetting = true }) { Text("Forget my key…", color = t.danger) }
                }
            }
        }
    }
    if (forgetting) {
        Confirm(
            "Forget your key?",
            "Thrall can't answer you until you add one again.",
            "Forget",
            onYes = { setKey(null) },
            onDone = { forgetting = false },
        )
    }
}

/** US dollars, to the cent. */
private fun dollars(usd: Double): String = String.format(Locale.US, "$%.2f", usd)

/** The start of this month where the phone is (Unix ms), for this month's spending. */
private fun monthStart(): Long =
    ZonedDateTime.now().withDayOfMonth(1).truncatedTo(ChronoUnit.DAYS).toInstant().toEpochMilli()

/** The value at a dotted key (`heartbeat.enabled`) in the config, if it's a plain value. */
private fun valueAt(config: JsonObject, key: String): JsonPrimitive? {
    val (section, name) = key.split('.', limit = 2).takeIf { it.size == 2 } ?: return null
    return runCatching { config[section]?.jsonObject?.get(name)?.jsonPrimitive }.getOrNull()
}

/** The settings shown on the phone, by section: (dotted key, label). Others stay on the computer. */
private val PHONE_SETTINGS: List<Pair<String, List<Pair<String, String>>>> = listOf(
    "Model" to listOf("provider.model" to "Model (an OpenRouter id, like openai/gpt-6-luna)"),
    "Check-ins" to listOf(
        "heartbeat.enabled" to "Check in now and then",
        "heartbeat.every_minutes" to "Every (minutes)",
        "heartbeat.active_hours" to "Only between (like 08:00-22:00; empty: any time)",
    ),
    "Memory" to listOf(
        "memory.save_facts" to "Save facts about you without asking (you're told each time)",
        "memory.remember_conversations" to "Remember conversations",
    ),
    "Home" to listOf(
        "home.fresh_start" to "Fresh start (daily, weekly or never)",
        "home.keep_days" to "Keep earlier pages (days; 0 keeps them)",
    ),
)

/** Characters of a fact shown when asking to forget it. */
private const val FACT_PREVIEW = 80
