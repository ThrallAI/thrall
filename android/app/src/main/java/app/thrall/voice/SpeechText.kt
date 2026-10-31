package app.thrall.voice

import app.thrall.ui.tableCells

/** Turning Thrall's markdown into something to say, and what was said into something to send. Pure, so it's tested on its own. */
object SpeechText {
    private val FENCE = Regex("^\\s*```")
    private val RULE = Regex("^\\|?\\s*:?-+:?\\s*(\\|\\s*:?-+:?\\s*)*\\|?$")
    private val LINK = Regex("\\[([^\\]]+)]\\((https?://[^)]+)\\)")
    private val URL = Regex("https?://\\S+")
    private val EMPHASIS_UNDERSCORE = Regex("(^|\\s)_+|_+(\\s|$|[.,;:!?])")
    private val SLASH = Regex("^(?:forward )?slash\\s+(\\S+)(.*)$", RegexOption.IGNORE_CASE)

    /**
     * What to read aloud: the words, without markdown's marks. Code blocks are left out (nobody
     * wants code read to them), links say their text, a table's rows are read cell by cell.
     */
    fun spoken(markdown: String): String {
        val out = ArrayList<String>()
        var inCode = false
        for (raw in markdown.replace("\r\n", "\n").lines()) {
            if (FENCE.containsMatchIn(raw)) {
                inCode = !inCode
                continue
            }
            if (inCode) continue
            var line = raw.trim()
            if (RULE.matches(line) && line.contains('-')) continue
            if (line.startsWith("|")) line = tableCells(line).filter { it.isNotEmpty() }.joinToString(", ")
            line = line.trimStart('#', '>').trim()
            line = line.replace(Regex("^[-*+] "), "")
            line = LINK.replace(line) { it.groupValues[1] }
            line = URL.replace(line, "a link")
            line = line.replace("`", "").replace("**", "").replace("__", "").replace("*", "").replace("~~", "")
            line = EMPHASIS_UNDERSCORE.replace(line) { it.groupValues[1] + it.groupValues[2] }
            if (line.isNotBlank()) out += line.trim()
        }
        return out.joinToString("\n")
    }

    /** The text in pieces no longer than [max], broken at a line or sentence where it can be. */
    fun pieces(text: String, max: Int): List<String> {
        val out = ArrayList<String>()
        var rest = text.trim()
        while (rest.length > max) {
            val window = rest.substring(0, max)
            val cut = listOf(window.lastIndexOf('\n'), window.lastIndexOf(". "), window.lastIndexOf(' '))
                .firstOrNull { it > max / 2 }?.plus(1) ?: max
            out += rest.substring(0, cut).trim()
            rest = rest.substring(cut).trim()
        }
        if (rest.isNotEmpty()) out += rest
        return out
    }

    /** What was dictated, as it goes in the box: "slash status" becomes `/status`. */
    fun dictated(text: String): String {
        val m = SLASH.matchEntire(text.trim()) ?: return text
        return "/" + m.groupValues[1].lowercase() + m.groupValues[2]
    }
}
