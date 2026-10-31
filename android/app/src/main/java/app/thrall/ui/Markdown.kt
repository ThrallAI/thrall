package app.thrall.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.data.FileKinds

/** The markdown blocks the app draws: headings, paragraphs, lists, quotes and code. */
sealed interface Block {
    data class Heading(val level: Int, val text: String) : Block
    data class Paragraph(val text: String) : Block
    data class Bullet(val text: String, val number: String?) : Block
    data class Quote(val text: String) : Block
    data class Code(val text: String) : Block
    /** A table: the header cells, then each row's cells (as many as the header). */
    data class Table(val header: List<String>, val rows: List<List<String>>) : Block
}

/** A table's line under the header: `|---|:--:|`. */
private val TABLE_RULE = Regex("^\\|?\\s*:?-+:?\\s*(\\|\\s*:?-+:?\\s*)*\\|?$")

/** One table line's cells: `| a | b |` → [a, b]. */
fun tableCells(line: String): List<String> =
    line.trim().removePrefix("|").removeSuffix("|").split('|').map { it.trim() }

/** Split markdown into blocks (no HTML: it shows as text). */
fun blocks(markdown: String): List<Block> {
    val out = ArrayList<Block>()
    val para = StringBuilder()
    fun flush() {
        if (para.isNotBlank()) out += Block.Paragraph(para.toString().trim())
        para.clear()
    }
    val lines = markdown.replace("\r\n", "\n").lines()
    var i = 0
    while (i < lines.size) {
        val line = lines[i]
        val trimmed = line.trimStart()
        when {
            trimmed.startsWith("```") -> {
                flush()
                val code = StringBuilder()
                i++
                while (i < lines.size && !lines[i].trimStart().startsWith("```")) {
                    code.appendLine(lines[i])
                    i++
                }
                out += Block.Code(code.toString().trimEnd('\n'))
            }
            trimmed.startsWith("|") && i + 1 < lines.size && TABLE_RULE.matches(lines[i + 1].trim()) -> {
                flush()
                val header = tableCells(trimmed)
                val rows = ArrayList<List<String>>()
                i += 2
                while (i < lines.size && lines[i].trimStart().startsWith("|")) {
                    val cells = tableCells(lines[i])
                    rows += List(header.size) { cells.getOrElse(it) { "" } }
                    i++
                }
                out += Block.Table(header, rows)
                continue
            }
            trimmed.startsWith("#") && trimmed.takeWhile { it == '#' }.length <= 6 && trimmed.drop(trimmed.takeWhile { it == '#' }.length).startsWith(" ") -> {
                flush()
                val level = trimmed.takeWhile { it == '#' }.length
                out += Block.Heading(level, trimmed.drop(level).trim())
            }
            Regex("^[-*+] ").containsMatchIn(trimmed) -> {
                flush()
                out += Block.Bullet(trimmed.drop(2).trim(), null)
            }
            Regex("^\\d+[.)] ").containsMatchIn(trimmed) -> {
                flush()
                val n = trimmed.takeWhile { it.isDigit() }
                out += Block.Bullet(trimmed.drop(n.length + 2).trim(), n)
            }
            trimmed.startsWith(">") -> {
                flush()
                out += Block.Quote(trimmed.drop(1).trim())
            }
            line.isBlank() -> flush()
            else -> para.append(line).append('\n')
        }
        i++
    }
    flush()
    return out
}

/**
 * Inline markdown: `code`, **bold**, *italic* / _italic_ and [links](https://…). With [onPath], a
 * code span that [pathOf] turns into a file in the session's folder (`out/report.md`, or the whole
 * path to it) can be tapped, and [onPath] gets that file's path inside the folder.
 */
fun inline(
    text: String,
    code: SpanStyle,
    link: SpanStyle,
    onPath: ((String) -> Unit)? = null,
    pathOf: (String) -> String? = { if (FileKinds.looksLikePath(it)) it else null },
): AnnotatedString = buildAnnotatedString {
    var i = 0
    while (i < text.length) {
        val c = text[i]
        when {
            c == '`' -> {
                val end = text.indexOf('`', i + 1)
                if (end > i) {
                    val span = text.substring(i + 1, end)
                    val path = if (onPath != null) pathOf(span) else null
                    if (onPath != null && path != null) {
                        withLink(LinkAnnotation.Clickable(span, linkInteractionListener = { onPath(path) })) {
                            withStyle(code.merge(link)) { append(span) }
                        }
                    } else {
                        withStyle(code) { append(span) }
                    }
                    i = end + 1
                    continue
                }
            }
            text.startsWith("**", i) -> {
                val end = text.indexOf("**", i + 2)
                if (end > i + 2) {
                    withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(inline(text.substring(i + 2, end), code, link, onPath, pathOf)) }
                    i = end + 2
                    continue
                }
            }
            (c == '*' || c == '_') && i + 1 < text.length && text[i + 1] != ' ' -> {
                val end = text.indexOf(c, i + 1)
                if (end > i + 1) {
                    withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(text.substring(i + 1, end)) }
                    i = end + 1
                    continue
                }
            }
            c == '[' -> {
                val close = text.indexOf("](", i)
                val end = if (close > 0) text.indexOf(')', close) else -1
                val url = if (end > 0) text.substring(close + 2, end) else ""
                if (end > 0 && (url.startsWith("https://") || url.startsWith("http://"))) {
                    withLink(LinkAnnotation.Url(url)) { withStyle(link) { append(text.substring(i + 1, close)) } }
                    i = end + 1
                    continue
                }
            }
        }
        append(c)
        i++
    }
}

@Composable
fun Markdown(
    text: String,
    modifier: Modifier = Modifier,
    style: TextStyle = TextStyle(fontSize = 15.sp, lineHeight = 22.sp),
    onPath: ((String) -> Unit)? = null,
    pathOf: (String) -> String? = { if (FileKinds.looksLikePath(it)) it else null },
) {
    val t = LocalTokens.current
    val code = SpanStyle(fontFamily = FontFamily.Monospace, color = t.code, background = t.raised)
    val link = SpanStyle(color = t.accent)
    SelectionContainer(modifier) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (b in blocks(text)) {
                when (b) {
                    is Block.Heading -> Text(
                        inline(b.text, code, link, onPath, pathOf),
                        style = style.copy(fontSize = (20 - b.level * 1.5).coerceAtLeast(15.0).sp, fontWeight = FontWeight.SemiBold),
                        color = t.ink,
                    )
                    is Block.Paragraph -> Text(inline(b.text, code, link, onPath, pathOf), style = style, color = t.ink)
                    is Block.Bullet -> Row {
                        Text(if (b.number != null) "${b.number}." else "•", style = style, color = t.inkMuted, modifier = Modifier.padding(end = 8.dp))
                        Text(inline(b.text, code, link, onPath, pathOf), style = style, color = t.ink)
                    }
                    is Block.Quote -> Text(
                        inline(b.text, code, link, onPath, pathOf),
                        style = style.copy(fontStyle = FontStyle.Italic),
                        color = t.inkMuted,
                        modifier = Modifier.padding(start = 12.dp),
                    )
                    is Block.Code -> Text(
                        b.text,
                        style = TextStyle(fontFamily = FontFamily.Monospace, fontSize = 13.sp, lineHeight = 19.sp),
                        color = t.ink,
                        softWrap = false,
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(t.raised, RoundedCornerShape(8.dp))
                            .horizontalScroll(rememberScrollState())
                            .padding(12.dp),
                    )
                    is Block.Table -> Table(b, style, code, link, onPath, pathOf)
                }
            }
        }
    }
}

/** A table as a grid, each column as wide as its longest cell (within limits); wide ones scroll sideways. */
@Composable
private fun Table(
    b: Block.Table,
    style: TextStyle,
    code: SpanStyle,
    link: SpanStyle,
    onPath: ((String) -> Unit)?,
    pathOf: (String) -> String?,
) {
    val t = LocalTokens.current
    val widths = b.header.indices.map { col ->
        val longest = (listOf(b.header) + b.rows).maxOf { it.getOrElse(col) { "" }.length }
        (longest * TABLE_DP_PER_CHAR).coerceIn(TABLE_MIN_DP, TABLE_MAX_DP).dp
    }
    val cell = style.copy(fontSize = 14.sp, lineHeight = 20.sp)
    Column(
        Modifier
            .border(1.dp, t.line, RoundedCornerShape(8.dp))
            .horizontalScroll(rememberScrollState()),
    ) {
        for ((r, row) in (listOf(b.header) + b.rows).withIndex()) {
            Row(Modifier.background(if (r == 0) t.accentSoft else if (r % 2 == 0) t.surface else t.bg)) {
                for ((c, text) in row.withIndex()) {
                    Text(
                        inline(text, code, link, onPath, pathOf),
                        style = if (r == 0) cell.copy(fontWeight = FontWeight.SemiBold) else cell,
                        color = t.ink,
                        modifier = Modifier.width(widths[c]).padding(horizontal = 10.dp, vertical = 8.dp),
                    )
                }
            }
        }
    }
}

/** Table column widths: room per character of the longest cell, and the narrowest and widest a column gets. */
private const val TABLE_DP_PER_CHAR = 8
private const val TABLE_MIN_DP = 64
private const val TABLE_MAX_DP = 220
