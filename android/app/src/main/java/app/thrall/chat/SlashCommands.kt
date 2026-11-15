package app.thrall.chat

import app.thrall.protocol.CommandItem

/** What a typed `/name args` is: run by the engine, or sent to Thrall as a message (skills). */
object SlashCommands {
    data class Typed(val name: String, val args: String)

    /**
     * `/model openai/gpt-6-luna` → (model, openai/gpt-6-luna). Not a command: text without a leading
     * slash, a bare `/`, or a path like `/home/me` (a name is letters, digits, `-` and `_`).
     */
    fun parse(text: String): Typed? {
        val t = text.trim()
        if (!t.startsWith("/")) return null
        val name = t.drop(1).takeWhile { !it.isWhitespace() }
        if (name.isEmpty() || !name.all { it.isLetterOrDigit() || it == '-' || it == '_' }) return null
        return Typed(name.lowercase(), t.drop(1 + name.length).trim())
    }

    /** Whether the engine runs it (a skill, or a name the list doesn't have yet, isn't). */
    fun runsOnEngine(typed: Typed, known: List<CommandItem>): Boolean =
        known.firstOrNull { it.name == typed.name }?.let { !it.skill } ?: false

    /**
     * What to say for a command only the terminal app has (`/settings`, `/quit`…), instead of
     * sending it to Thrall as a message. Null for anything else, or a name the engine knows.
     */
    fun terminalOnly(typed: Typed, known: List<CommandItem>): String? {
        if (known.any { it.name == typed.name }) return null
        val where = TERMINAL_ONLY[typed.name] ?: return null
        return "`/${typed.name}` is a terminal command. $where"
    }

    /** Commands the terminal app runs itself, and where the phone has the same thing. */
    val TERMINAL_ONLY = mapOf(
        "settings" to "Here it's Settings… in the chat list's ⋮ menu.",
        "group" to "Here it's Groups… in the chat list's ⋮ menu, or hold a chat to move it.",
        "forget" to "Here it's What Thrall remembers… in the chat list's ⋮ menu.",
        "skills" to "Skills are in the desktop app's Settings → Learning.",
        "skill" to "Skills are in the desktop app's Settings → Learning.",
        "lessons" to "Lessons are in the desktop app's Settings → Learning.",
        "home" to "Here Home is at the top of the chat list.",
        "new" to "Here it's New chat in the chat list.",
        "resume" to "Here every chat is in the chat list.",
        "retry" to "Here, hold your last message and pick Send again.",
        "edit" to "Here, hold your last message and pick Edit and send again.",
        "rewind" to "Here, hold the message and pick Rewind to before this…",
        "expand" to "Here, tap a step to see its details.",
        "quit" to "Here, just close the app.",
    )

    /** Commands to offer while `/…` is being typed (before the first space). */
    fun suggestions(text: String, known: List<CommandItem>): List<CommandItem> {
        if (!text.startsWith("/") || text.any { it.isWhitespace() }) return emptyList()
        val typed = text.drop(1).lowercase()
        return known.filter { it.name.startsWith(typed) }.take(SUGGESTIONS_MAX)
    }

    const val SUGGESTIONS_MAX = 6
}
