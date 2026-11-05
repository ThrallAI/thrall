package app.thrall.protocol

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * The engine's protocol (docs/protocol.md), as the app uses it. Requests and replies are tagged by
 * `type`; anything the app doesn't know yet is parsed as "unknown" and left alone, so a newer engine
 * never breaks it. [ProtocolTypes] lists every type the schema has, so the drift test notices new ones.
 */
object Protocol {
    const val VERSION = 1

    val json = Json {
        classDiscriminator = "type"
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = false
    }

    /** A request as the text of one frame. */
    fun frame(id: String, request: Request): String {
        val body = json.encodeToJsonElement(Request.serializer(), request).jsonObject
        val obj = buildJsonObject {
            put("v", JsonPrimitive(VERSION))
            put("id", JsonPrimitive(id))
            body.forEach { (k, v) -> put(k, v) }
        }
        return obj.toString()
    }

    /** One frame from the engine: a reply (it has `re`) or a session event. */
    fun parse(text: String): Incoming? {
        val obj = try {
            json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            return null
        }
        val re = obj["re"]?.jsonPrimitive?.content
        if (re != null) {
            val reply = decode(obj) { json.decodeFromJsonElement(Reply.serializer(), it) } ?: Reply.Unknown
            return Incoming.ReplyFrame(re, reply)
        }
        val session = obj["session"]?.jsonPrimitive?.content ?: return null
        val event = decode(obj) { json.decodeFromJsonElement(Event.serializer(), it) } ?: Event.Unknown
        return Incoming.EventFrame(session, obj["seq"]?.jsonPrimitive?.longOrNull, event)
    }

    private fun <T> decode(obj: JsonObject, f: (JsonObject) -> T): T? = try {
        f(obj)
    } catch (e: SerializationException) {
        null
    } catch (e: IllegalArgumentException) {
        null
    }
}

sealed interface Incoming {
    data class ReplyFrame(val re: String, val reply: Reply) : Incoming
    data class EventFrame(val session: String, val seq: Long?, val event: Event) : Incoming
}

/** One question on a question card: the person can pick an option or type their own answer. */
@Serializable
data class QuestionItem(val text: String, val options: List<String> = emptyList(), val recommended: String)

@Serializable
data class ClientInfo(val kind: String, val name: String, val version: String)

@Serializable
data class EngineInfo(val version: String, val model: String = "")

/** Who the engine acts for on this connection: [role] is `owner` (runs this Thrall) or `member`. */
@Serializable
data class UserInfo(val id: String, val name: String, val role: String = "") {
    /** Runs this Thrall (an engine too old to say counts as the owner: it had nobody else). */
    val owner get() = role.isEmpty() || role == "owner"
}

/** A person, as `people.listed` has them; the phone asks only for its own person. */
@Serializable
data class PersonInfo(
    val person: String,
    val name: String,
    val role: String = "",
    /** What their turns cost (US$) since the time asked about. */
    val spent: Double = 0.0,
    /** Their turns use their own AI key, not the owner's. */
    @SerialName("own_key") val ownKey: Boolean = false,
    /** Their own key is kept (it's never shown). */
    @SerialName("key_set") val keySet: Boolean = false,
    val model: String? = null,
    /** The most their turns may cost in a calendar month (US$). */
    @SerialName("monthly_limit") val monthlyLimit: Double? = null,
)

@Serializable
data class Limits(@SerialName("frame_bytes") val frameBytes: Long)

/** A file a step made or changed, as the session's folder shows it (`kind`: create · modify · delete). */
@Serializable
data class Diff(val path: String, val kind: String, val added: Int = 0, val removed: Int = 0)

@Serializable
data class FileRef(val file: String, val name: String, val kind: String = "other", val size: Long = 0)

@Serializable
data class Usage(
    @SerialName("input_tokens") val inputTokens: Long = 0,
    @SerialName("output_tokens") val outputTokens: Long = 0,
    @SerialName("cost_usd") val costUsd: Double? = null,
)

@Serializable
data class SessionInfo(
    val session: String,
    val title: String,
    val workspace: String? = null,
    val mode: String = "ask",
    val state: String = "idle",
    @SerialName("last_seq") val lastSeq: Long = 0,
    val updated: Long = 0,
    @SerialName("cost_usd") val costUsd: Double = 0.0,
    val group: String? = null,
    val pinned: Boolean = false,
    val home: Boolean = false,
    @SerialName("mode_limit") val modeLimit: ModeLimit? = null,
)

/** How long Full auto lasts: [until] is `task_end`, `time` (until [endsAt], Unix ms) or `manual`. */
@Serializable
data class ModeLimit(val until: String, @SerialName("ends_at") val endsAt: Long? = null)

@OptIn(ExperimentalSerializationApi::class)
@Serializable
sealed class Request {
    @Serializable @SerialName("hello")
    data class Hello(
        val protocols: List<Int>,
        val client: ClientInfo,
        /** Required by the engine; ignored on the phone link (the key proves who this is). */
        @EncodeDefault val token: String = "",
    ) : Request()

    @Serializable @SerialName("ping")
    data object Ping : Request()

    @Serializable @SerialName("session.list")
    data class SessionList(val limit: Int? = null, val archived: Boolean? = null) : Request()

    @Serializable @SerialName("session.open")
    data class SessionOpen(val session: String, @SerialName("after_seq") val afterSeq: Long? = null) : Request()

    @Serializable @SerialName("session.detach")
    data class SessionDetach(val session: String) : Request()

    @Serializable @SerialName("session.create")
    data class SessionCreate(val workspace: String? = null, val title: String? = null) : Request()

    @Serializable @SerialName("home.get")
    data object HomeGet : Request()

    @Serializable @SerialName("turn.send")
    data class TurnSend(val session: String, val text: String, val files: List<String> = emptyList()) : Request()

    @Serializable @SerialName("turn.interrupt")
    data class TurnInterrupt(val session: String) : Request()

    /** Answer a permission card: `yes`, `no` or `always`. */
    @Serializable @SerialName("permission.answer")
    data class PermissionAnswer(val request: String, val choice: String) : Request()

    /** Approve or turn down a plan; `auto_edits` also lets its edits through without asking. */
    @Serializable @SerialName("plan.decide")
    data class PlanDecide(
        val plan: String,
        val approve: Boolean,
        @EncodeDefault @SerialName("auto_edits") val autoEdits: Boolean = false,
    ) : Request()

    /** Answer a question card: one answer per question, in order. */
    @Serializable @SerialName("question.answer")
    data class QuestionAnswer(val question: String, val answers: List<String>) : Request()

    /** Events from every session (live text left out), for the list's "Needs you" badges. */
    @Serializable @SerialName("session.watch_all")
    data object SessionWatchAll : Request()

    /** A folder inside a session's folder ("" for the folder itself). */
    @Serializable @SerialName("folder.list")
    data class FolderList(val session: String, @EncodeDefault val path: String = "") : Request()

    /** A piece of a file inside a session's folder. */
    @Serializable @SerialName("folder.read")
    data class FolderRead(val session: String, val path: String, @EncodeDefault val offset: Long = 0) : Request()

    /** A piece of a file attached to a message or made by a step. */
    @Serializable @SerialName("file.get")
    data class FileGet(val file: String, @EncodeDefault val offset: Long = 0) : Request()

    /** Folders a new chat could work in. */
    @Serializable @SerialName("folder.places")
    data object FolderPlaces : Request()

    /**
     * A piece of a file sent from the phone; `data` is base64. Without [upload] it starts a new file,
     * with it (from this connection's `file.saved`) it adds to the end.
     */
    @Serializable @SerialName("file.put")
    data class FilePut(val upload: String? = null, val name: String, val data: String) : Request()

    /** The slash commands, for the list shown while typing `/`. */
    @Serializable @SerialName("command.list")
    data object CommandList : Request()

    /** Run a slash command (not a skill: those go as messages). */
    @Serializable @SerialName("command.run")
    data class CommandRun(val session: String?, val name: String, @EncodeDefault val args: String = "") : Request()

    /** A new empty folder in one of the folders `folder.places` offers. */
    @Serializable @SerialName("folder.create")
    data class FolderCreate(val parent: String, val name: String) : Request()

    @Serializable @SerialName("session.rename")
    data class SessionRename(val session: String, val title: String) : Request()

    @Serializable @SerialName("session.pin")
    data class SessionPin(val session: String, @EncodeDefault val pinned: Boolean) : Request()

    /** Hide a chat from the list (kept in full), or bring it back. */
    @Serializable @SerialName("session.archive")
    data class SessionArchive(val session: String, @EncodeDefault val archived: Boolean) : Request()

    /** Delete a chat for good (the app asks first; the engine doesn't). */
    @Serializable @SerialName("session.delete")
    data class SessionDelete(val session: String) : Request()

    /** `ask`, `auto_edits` or `plan` (`full_auto` is only ever set on the computer). */
    @Serializable @SerialName("session.set_mode")
    data class SessionSetMode(
        val session: String,
        val mode: String,
        /** Full auto only: `task_end`, `time` (with [minutes]) or `manual`. */
        val until: String? = null,
        val minutes: Int? = null,
    ) : Request()

    /** The newest phone app the engine offers. */
    @Serializable @SerialName("app.latest")
    data object AppLatest : Request()

    /** A piece of that phone app (answered by `file.part`). */
    @Serializable @SerialName("app.get")
    data class AppGet(@EncodeDefault val offset: Long = 0) : Request()

    /** Take back the last turn and send its message again. */
    @Serializable @SerialName("turn.retry")
    data class TurnRetry(val session: String) : Request()

    /** Take back the last turn and send [text] instead. */
    @Serializable @SerialName("turn.edit")
    data class TurnEdit(val session: String, val text: String) : Request()

    /** What rewinding to before [turn] would do. */
    @Serializable @SerialName("checkpoint.preview")
    data class CheckpointPreview(val session: String, val turn: String) : Request()

    /** Go back to before [turn]: files put back (except [keep]) and/or the conversation taken back. */
    @Serializable @SerialName("turn.rewind")
    data class TurnRewind(
        val session: String,
        val turn: String,
        @EncodeDefault val files: Boolean,
        @EncodeDefault val conversation: Boolean,
        val keep: List<String> = emptyList(),
    ) : Request()

    @Serializable @SerialName("group.list")
    data object GroupList : Request()

    @Serializable @SerialName("group.create")
    data class GroupCreate(val name: String) : Request()

    @Serializable @SerialName("group.rename")
    data class GroupRename(val group: String, val name: String) : Request()

    /** Delete a group; its chats stay, ungrouped. */
    @Serializable @SerialName("group.delete")
    data class GroupDelete(val group: String) : Request()

    @Serializable @SerialName("group.reorder")
    data class GroupReorder(val groups: List<String>) : Request()

    /** Put a chat in a group, or take it out ([group] null). */
    @Serializable @SerialName("session.move")
    data class SessionMove(val session: String, val group: String? = null) : Request()

    /** Delete every Home page (the app asks first). */
    @Serializable @SerialName("home.clear")
    data object HomeClear : Request()

    @Serializable @SerialName("heartbeat.get")
    data object HeartbeatGet : Request()

    /** Replace the check-in checklist (empty removes it). */
    @Serializable @SerialName("heartbeat.set")
    data class HeartbeatSet(@EncodeDefault val checklist: String) : Request()

    /** Check in now. */
    @Serializable @SerialName("heartbeat.run")
    data object HeartbeatRun : Request()

    @Serializable @SerialName("memory.list")
    data object MemoryList : Request()

    @Serializable @SerialName("memory.forget")
    data class MemoryForget(val fact: String) : Request()

    /** Delete one of the person's own skills. */
    @Serializable @SerialName("skill.remove")
    data class SkillRemove(val name: String) : Request()

    @Serializable @SerialName("settings.get")
    data object SettingsGet : Request()

    /** Change one setting; [value] is a yes/no, a whole number or text. */
    @Serializable @SerialName("settings.set")
    data class SettingsSet(val key: String, val value: kotlinx.serialization.json.JsonPrimitive) : Request()

    /** Your personality: how Thrall talks to you. */
    @Serializable @SerialName("personality.get")
    data object PersonalityGet : Request()

    /** Choose your personality: a preset's id, or `custom` with your own words. Only ever your own. */
    @Serializable @SerialName("personality.set")
    data class PersonalitySet(val preset: String, @EncodeDefault val custom: String = "") : Request()

    /** The people on this Thrall; anyone but the owner gets only themselves. [since]: count spending from (Unix ms). */
    @Serializable @SerialName("people.list")
    data class PeopleList(val since: Long? = null) : Request()

    /** Keep your own AI key; without [key], forget it. Write-only: it's never sent back. */
    @Serializable @SerialName("people.key")
    data class PeopleKey(val key: String? = null) : Request()
}

@Serializable
data class GroupInfo(val group: String, val name: String)

/** A file rewinding would touch: `restore` · `delete` · `unavailable`; [changed] since Thrall's last edit. */
@Serializable
data class RewindFile(val path: String, val action: String, val changed: Boolean = false)

/** A fact Thrall remembers. */
@Serializable
data class FactInfo(val fact: String, val topic: String = "", val content: String, val at: Long = 0)

/** A personality preset: its id, name and a line about it. */
@Serializable
data class PersonalityPreset(val id: String, val name: String, val description: String = "")

@Serializable
data class SkillInfo(
    val name: String,
    val description: String = "",
    val personal: Boolean = false,
    val helper: Boolean = false,
    val enabled: Boolean = true,
    /** Safety rating of a skill Thrall learned: `low`, `medium` or `high`. */
    val rating: String? = null,
)

/** A build of this app the engine offers as an update. */
@Serializable
data class AppRelease(
    @SerialName("version_code") val versionCode: Int,
    @SerialName("version_name") val versionName: String,
    val size: Long,
    val sha256: String,
    val notes: String = "",
)

/** A slash command; [skill] ones are sent to Thrall as a message instead of run. */
@Serializable
data class CommandItem(
    val name: String,
    val summary: String = "",
    @SerialName("args_hint") val argsHint: String = "",
    val skill: Boolean = false,
)

/** One entry in a folder listing. */
@Serializable
data class FolderEntry(val name: String, val folder: Boolean = false, val size: Long = 0, val modified: Long = 0)

@Serializable
sealed class Reply {
    @Serializable @SerialName("welcome")
    data class Welcome(
        val protocol: Int,
        val engine: EngineInfo,
        val user: UserInfo,
        val capabilities: List<String> = emptyList(),
        val limits: Limits,
    ) : Reply()

    @Serializable @SerialName("pong")
    data object Pong : Reply()

    @Serializable @SerialName("ack")
    data object Ack : Reply()

    @Serializable @SerialName("error")
    data class Error(val code: String, val message: String, val retry: Boolean = false) : Reply()

    /** `session.opened`: the session's details sit beside `type` in the frame. */
    @Serializable @SerialName("session.opened")
    data class SessionOpened(
        val session: String,
        val title: String,
        val workspace: String? = null,
        val mode: String = "ask",
        val state: String = "idle",
        @SerialName("last_seq") val lastSeq: Long = 0,
        val updated: Long = 0,
        @SerialName("cost_usd") val costUsd: Double = 0.0,
        val home: Boolean = false,
        @SerialName("mode_limit") val modeLimit: ModeLimit? = null,
    ) : Reply() {
        val info get() = SessionInfo(session, title, workspace, mode, state, lastSeq, updated, costUsd, home = home, modeLimit = modeLimit)
    }

    @Serializable @SerialName("session.listed")
    data class SessionListed(val items: List<SessionInfo>) : Reply()

    @Serializable @SerialName("home.shown")
    data class HomeShown(val page: SessionInfo, val earlier: List<SessionInfo> = emptyList()) : Reply()

    @Serializable @SerialName("folder.listed")
    data class FolderListed(
        val session: String,
        val path: String = "",
        val items: List<FolderEntry> = emptyList(),
        val more: Boolean = false,
    ) : Reply()

    /** A piece of a file in a session's folder; `data` is base64. */
    @Serializable @SerialName("folder.part")
    data class FolderPart(val session: String, val path: String, val offset: Long, val size: Long, val data: String) : Reply()

    /** A piece of an attached file; `data` is base64. */
    @Serializable @SerialName("file.part")
    data class FilePart(val file: String, val name: String, val offset: Long, val size: Long, val data: String) : Reply()

    @Serializable @SerialName("folder.places")
    data class FolderPlaces(
        val recent: List<String> = emptyList(),
        val default: String? = null,
        /** Where a new folder goes unless another is picked. */
        @SerialName("new_in") val newIn: String? = null,
    ) : Reply()

    @Serializable @SerialName("folder.created")
    data class FolderCreated(val path: String) : Reply()

    @Serializable @SerialName("app.latest")
    data class AppLatest(val release: AppRelease? = null) : Reply()

    @Serializable @SerialName("group.listed")
    data class GroupListed(val items: List<GroupInfo> = emptyList()) : Reply()

    @Serializable @SerialName("heartbeat.shown")
    data class HeartbeatShown(val checklist: String = "", @SerialName("next_run") val nextRun: Long? = null) : Reply()

    @Serializable @SerialName("checkpoint.shown")
    data class CheckpointShown(val turn: String, val text: String = "", val files: List<RewindFile> = emptyList()) : Reply()

    /** After a rewind: [text] is the message to put back in the box. */
    @Serializable @SerialName("turn.rewound")
    data class TurnRewound(val text: String = "", val restored: List<String> = emptyList(), val failed: List<String> = emptyList()) : Reply()

    @Serializable @SerialName("memory.listed")
    data class MemoryListed(val facts: List<FactInfo> = emptyList()) : Reply()

    /** The settings, as the config file has them (secrets hidden), the model in use, and skills. */
    @Serializable @SerialName("personality.shown")
    data class PersonalityShown(val preset: String, val custom: String = "", val presets: List<PersonalityPreset> = emptyList()) : Reply()

    @Serializable @SerialName("settings.shown")
    data class SettingsShown(
        val config: JsonObject = JsonObject(emptyMap()),
        val model: String = "",
        val skills: List<SkillInfo> = emptyList(),
        val pending: Boolean = false,
    ) : Reply()

    /** A sent piece was kept: [file] is the upload's id, for more pieces and for `turn.send`. */
    @Serializable @SerialName("file.saved")
    data class FileSaved(val file: String, val name: String = "", val size: Long = 0) : Reply()

    @Serializable @SerialName("command.listed")
    data class CommandListed(val items: List<CommandItem> = emptyList()) : Reply()

    @Serializable @SerialName("command.result")
    data class CommandResult(val markdown: String) : Reply()

    @Serializable @SerialName("people.listed")
    data class PeopleListed(val items: List<PersonInfo> = emptyList()) : Reply()

    /** A reply this version of the app doesn't read (never sent by the engine under this name). */
    @Serializable @SerialName("app.unknown")
    data object Unknown : Reply()
}

@Serializable
sealed class Event {
    @Serializable @SerialName("turn.started")
    data class TurnStarted(val turn: String, val scheduled: Boolean = false) : Event()

    @Serializable @SerialName("turn.completed")
    data class TurnCompleted(val turn: String, val status: String, val usage: Usage = Usage()) : Event()

    @Serializable @SerialName("turn.retracted")
    data class TurnRetracted(val turn: String) : Event()

    @Serializable @SerialName("message.started")
    data class MessageStarted(val turn: String, val message: String, val role: String) : Event()

    @Serializable @SerialName("message.delta")
    data class MessageDelta(val message: String, val text: String) : Event()

    @Serializable @SerialName("reasoning.delta")
    data class ReasoningDelta(val message: String, val text: String) : Event()

    @Serializable @SerialName("message.completed")
    data class MessageCompleted(val message: String, val text: String, val files: List<FileRef> = emptyList()) : Event()

    @Serializable @SerialName("step.started")
    data class StepStarted(val turn: String, val step: String, val tool: String, val title: String) : Event()

    @Serializable @SerialName("step.output")
    data class StepOutput(val step: String, val text: String) : Event()

    @Serializable @SerialName("step.completed")
    data class StepCompleted(
        val step: String,
        val status: String,
        val summary: String,
        val files: List<FileRef> = emptyList(),
        val diff: Diff? = null,
    ) : Event()

    @Serializable @SerialName("permission.request")
    data class PermissionRequest(
        val request: String,
        val turn: String,
        val step: String,
        val tool: String,
        val summary: String,
        val risk: String,
        /** The answers on offer (`yes`, `no`, and `always` when it may be offered). */
        val choices: List<String> = listOf("yes", "no"),
    ) : Event()

    @Serializable @SerialName("permission.resolved")
    data class PermissionResolved(val request: String, val choice: String, @SerialName("answered_by") val answeredBy: String) : Event()

    @Serializable @SerialName("plan.proposed")
    data class PlanProposed(val plan: String, val markdown: String) : Event()

    @Serializable @SerialName("plan.resolved")
    data class PlanResolved(val plan: String, val approved: Boolean, @SerialName("answered_by") val answeredBy: String) : Event()

    /** Thrall's questions, each with its recommended answer; nobody answering by [expiresAt] means those are used. */
    @Serializable @SerialName("question.ask")
    data class QuestionAsk(
        val question: String,
        val items: List<QuestionItem>,
        val why: String = "",
        @SerialName("expires_at") val expiresAt: Long = 0,
    ) : Event()

    @Serializable @SerialName("question.resolved")
    data class QuestionResolved(val question: String, val answers: List<String>, @SerialName("answered_by") val answeredBy: String) : Event()

    @Serializable @SerialName("session.updated")
    data class SessionUpdated(
        val title: String? = null,
        val mode: String? = null,
        @SerialName("mode_limit") val modeLimit: ModeLimit? = null,
        val workspace: String? = null,
    ) : Event()

    @Serializable @SerialName("status")
    data class Status(val state: String, val model: String = "") : Event()

    @Serializable @SerialName("notice")
    data class Notice(val level: String, val text: String) : Event()

    @Serializable @SerialName("error")
    data class Error(val code: String, val message: String, val retry: Boolean = false) : Event()

    @Serializable @SerialName("agent.completed")
    data class AgentCompleted(val agent: String, val title: String, val status: String, val summary: String) : Event()

    /** An event this version of the app doesn't show (never sent by the engine under this name). */
    @Serializable @SerialName("app.unknown")
    data object Unknown : Event()
}

/**
 * Every `type` in the protocol, as this app knows it: the ones it uses and the ones it knowingly
 * leaves for later. The drift test fails when the engine's schema has a type that's in neither.
 */
object ProtocolTypes {
    val requestsUsed = setOf(
        "hello", "ping", "session.list", "session.open", "session.detach", "session.create", "home.get",
        "turn.send", "turn.interrupt", "permission.answer", "plan.decide", "question.answer", "session.watch_all",
        "folder.list", "folder.read", "file.get", "folder.places", "command.list", "command.run", "file.put",
        "folder.create", "session.rename", "session.pin", "session.archive", "session.delete",
        "session.set_mode", "app.latest", "app.get",
        "session.move", "group.list", "group.create", "group.rename", "group.delete", "group.reorder",
        "home.clear", "heartbeat.get", "heartbeat.set", "heartbeat.run", "checkpoint.preview", "turn.rewind",
        "turn.retry", "turn.edit", "memory.list", "memory.forget", "skill.remove", "settings.get", "settings.set",
        "personality.get", "personality.set",
        "people.list", "people.key",
    )
    val requestsLater = setOf(
        // Stopping the engine is for the computer; the phone dictates with its own speech recognition.
        "engine.stop", "audio.transcribe",
        // Managed on the computer only: refused over the phone link.
        "phone.list", "phone.pair", "phone.assign", "phone.remove",
        "pc.requests", "pc.accept", "pc.decline", "pc.unblock",
        // Only a companion PC's runner sends these.
        "runner.ready", "runner.output", "runner.done",
        // Moving a chat to another folder: the phone has /folder for now.
        "session.set_folder",
        // People are managed on the computer's People page; the phone shows only its own person.
        "people.add", "people.rename", "people.remove", "people.terms",
        // Browser sign-in links and signed-in browsers: managed in the web app.
        "signin.create", "device.list", "device.remove",
        // Starting with Windows is a setting of the computer, changed in its Settings or tray.
        "startup.get", "startup.set",
        // The experience log is shown and edited in the computer's Settings → Learning for now.
        "experience.list", "experience.forget",
        // Reading, editing and making skills safe: in the computer's Settings → Learning for now.
        "skill.get", "skill.edit", "skill.make_safe",
        // Suppliers are set up and tested on the computer's Settings → Providers.
        "provider.test", "local.find",
        // Thrall's folders open in the computer's file manager, from its Settings.
        "folders.get", "folder.open",
        // Updates install through the computer's desktop app, from its Settings or tray.
        "update.install",
    )
    val repliesUsed = setOf(
        "welcome", "pong", "ack", "error", "session.opened", "session.listed", "home.shown", "folder.listed",
        "folder.part", "file.part", "folder.places", "command.listed", "command.result", "file.saved",
        "folder.created", "app.latest", "group.listed", "heartbeat.shown", "checkpoint.shown", "turn.rewound",
        "memory.listed", "settings.shown", "people.listed", "personality.shown",
    )
    val repliesLater = setOf("experience.listed", "skill.shown", "skill.safer", "audio.transcribed", "phone.listed", "phone.pairing", "signin.code", "device.listed", "startup.shown", "provider.tested", "local.found", "folders.shown", "pc.requests")
    val eventsUsed = setOf(
        "turn.started", "turn.completed", "turn.retracted", "message.started", "message.delta",
        "reasoning.delta", "message.completed", "step.started", "step.output", "step.completed",
        "permission.request", "permission.resolved", "plan.proposed", "plan.resolved", "question.ask",
        "question.resolved", "session.updated",
        "status", "notice", "error", "agent.completed",
    )
    val eventsLater = setOf("tasks.updated", "agent.started", "memory.saved", "update.requested", "pc.asked", "runner.call")
}
