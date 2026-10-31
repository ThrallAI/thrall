package app.thrall.chat

import app.thrall.protocol.GroupInfo
import app.thrall.protocol.SessionInfo

/**
 * The chat list laid out as the web app's sidebar does it: Pinned first, then each of the person's
 * groups (in their order, empty ones too), then every other chat by the folder it works in, those
 * sections ordered by their newest chat.
 */
object SessionLayout {
    data class Section(
        /** Stable id, for remembering which are folded. */
        val key: String,
        val label: String,
        val sessions: List<SessionInfo>,
        /** The folder's full path, for a folder section. */
        val folder: String? = null,
        /** The group, for a group section. */
        val group: GroupInfo? = null,
        /** Show each chat's folder under it (folder sections already say it). */
        val showFolder: Boolean = true,
    ) {
        /** Which part of the list it's in: pinned, the groups, or the folders. */
        val part: Part get() = when {
            key == PINNED -> Part.Pinned
            group != null -> Part.Groups
            else -> Part.Folders
        }
    }

    enum class Part { Pinned, Groups, Folders }

    const val PINNED = "pinned"
    const val NO_FOLDER_LABEL = "No folder"

    /** [sessions] newest first, as the engine lists them. */
    fun arrange(sessions: List<SessionInfo>, groups: List<GroupInfo>): List<Section> {
        val known = groups.map { it.group }.toSet()
        val pinned = sessions.filter { it.pinned }
        val loose = sessions.filter { !it.pinned && (it.group == null || it.group !in known) }
        val out = mutableListOf<Section>()
        if (pinned.isNotEmpty()) out += Section(PINNED, "Pinned", pinned)
        for (g in groups) {
            out += Section("group:${g.group}", g.name, sessions.filter { !it.pinned && it.group == g.group }, group = g)
        }
        // Folders keep the list's order, so the folder with the newest chat comes first.
        val byFolder = LinkedHashMap<String, MutableList<SessionInfo>>()
        for (s in loose) byFolder.getOrPut(s.workspace.orEmpty()) { mutableListOf() } += s
        for ((folder, chats) in byFolder) {
            out += Section(
                "folder:$folder",
                if (folder.isEmpty()) NO_FOLDER_LABEL else folderName(folder),
                chats,
                folder = folder.ifEmpty { null },
                showFolder = false,
            )
        }
        return out
    }

    /** The last part of a folder path (`C:\Users\me\Thrall` → `Thrall`). */
    fun folderName(path: String): String =
        path.trimEnd('\\', '/').substringAfterLast('\\').substringAfterLast('/').ifEmpty { path }
}
