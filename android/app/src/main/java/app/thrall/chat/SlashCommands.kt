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

    /** Commands to offer while `/…` is being typed (before the first space). */
    fun suggestions(text: String, known: List<CommandItem>): List<CommandItem> {
        if (!text.startsWith("/") || text.any { it.isWhitespace() }) return emptyList()
        val typed = text.drop(1).lowercase()
        return known.filter { it.name.startsWith(typed) }.take(SUGGESTIONS_MAX)
    }

    const val SUGGESTIONS_MAX = 6
}
