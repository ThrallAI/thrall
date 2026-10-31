package app.thrall.data

/** How the app shows a file: in its own viewer, or handed to another app on the phone. */
enum class FileKind { Markdown, Text, Image, Other }

object FileKinds {
    /** Largest text file shown in the app's own viewer (bigger ones go to another app). */
    const val TEXT_VIEW_MAX_BYTES = 2L * 1024 * 1024

    private val markdown = setOf("md", "markdown")
    private val text = setOf(
        "txt", "log", "csv", "tsv", "json", "jsonl", "toml", "yaml", "yml", "xml", "ini", "cfg", "conf",
        "py", "rs", "kt", "kts", "java", "js", "jsx", "ts", "tsx", "c", "h", "cpp", "hpp", "cs", "go", "rb",
        "php", "sh", "ps1", "bat", "sql", "html", "css", "scss", "gradle", "env", "gitignore", "lock",
    )
    private val images = setOf("png", "jpg", "jpeg", "webp", "gif", "bmp")

    /** Types other apps understand, for the ones Android's own table may not know. */
    private val mimes = mapOf(
        "pdf" to "application/pdf",
        "docx" to "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" to "application/msword",
        "xlsx" to "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" to "application/vnd.ms-excel",
        "pptx" to "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "dxf" to "image/vnd.dxf",
        "svg" to "image/svg+xml",
        "zip" to "application/zip",
        "mp3" to "audio/mpeg",
        "wav" to "audio/wav",
        "m4a" to "audio/mp4",
        "mp4" to "video/mp4",
        "mov" to "video/quicktime",
        "png" to "image/png",
        "jpg" to "image/jpeg",
        "jpeg" to "image/jpeg",
        "webp" to "image/webp",
        "gif" to "image/gif",
        "md" to "text/markdown",
        "txt" to "text/plain",
        "csv" to "text/csv",
        "json" to "application/json",
        "html" to "text/html",
    )

    fun extension(name: String): String = name.substringAfterLast('/').substringAfterLast('.', "").lowercase()

    fun kind(name: String, size: Long = 0): FileKind {
        val ext = extension(name)
        return when {
            ext in images -> FileKind.Image
            size > TEXT_VIEW_MAX_BYTES -> FileKind.Other
            ext in markdown -> FileKind.Markdown
            ext in text -> FileKind.Text
            else -> FileKind.Other
        }
    }

    /** The MIME type to hand another app, when known here. */
    fun mime(name: String): String? = mimes[extension(name)]

    /**
     * Whether `text` (from a code span in a reply) looks like a file in the session's folder:
     * a relative path ending in a file name with an extension, no spaces or schemes.
     */
    fun looksLikePath(text: String): Boolean {
        val t = text.trim()
        if (t.isEmpty() || t.length > MAX_PATH_CHARS) return false
        if (t.any { it.isWhitespace() } || "://" in t || t.startsWith("-")) return false
        if (t.contains(':') || t.startsWith("/") || t.startsWith("\\") || t.contains("..")) return false
        val last = t.substringAfterLast('/')
        val ext = last.substringAfterLast('.', "")
        return last.contains('.') && !last.startsWith('.') && ext.length in 1..MAX_EXT_CHARS &&
            ext.all { it.isLetterOrDigit() } && ext.any { it.isLetter() }
    }

    /**
     * `text` (a code span in a reply) as a path inside the session's [folder], or null when it isn't
     * a file there. Relative paths are taken as they are; a whole path (`C:\Users\…\notes.md`) counts
     * when it starts with the session's folder. Either slash works.
     */
    fun pathIn(text: String, folder: String?): String? {
        val t = text.trim().replace('\\', '/')
        val relative = if (t.startsWith("/") || t.contains(':')) {
            val base = folder?.replace('\\', '/')?.trimEnd('/')?.takeIf { it.isNotEmpty() } ?: return null
            if (!t.startsWith("$base/", ignoreCase = true)) return null
            t.substring(base.length + 1)
        } else {
            t.removePrefix("./")
        }
        return relative.takeIf { looksLikePath(it) }
    }

    private const val MAX_PATH_CHARS = 200
    private const val MAX_EXT_CHARS = 8
}
