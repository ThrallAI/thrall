package app.thrall.data

import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import java.io.File
import java.util.Base64

/** Where a file comes from: a session's folder, or an upload (attached to a message, made by a step). */
sealed interface FileSource {
    /** The file's name, for showing and for its type. */
    val name: String

    data class Folder(val session: String, val path: String) : FileSource {
        override val name: String get() = path.substringAfterLast('/')
    }

    data class Upload(val file: String, override val name: String) : FileSource
}

/**
 * Copies files from the computer into the app's cache, a piece at a time over the encrypted link,
 * written straight to disk. A copy is fetched again each time it's opened, so it's never stale.
 */
class Downloads(private val cache: File, private val connection: Connection) {
    /** Fetch [source] into the cache; [progress] gets (bytes so far, total). Returns the copy. */
    suspend fun fetch(source: FileSource, progress: (Long, Long) -> Unit = { _, _ -> }): File {
        val target = target(source)
        target.parentFile?.mkdirs()
        val part = File(target.path + PART_SUFFIX)
        part.outputStream().use { out ->
            var offset = 0L
            while (true) {
                val (start, size, data) = piece(source, offset)
                if (start != offset) throw RequestFailed("The file changed while it was being fetched.")
                val bytes = Base64.getDecoder().decode(data)
                out.write(bytes)
                offset += bytes.size
                progress(offset, size)
                if (offset >= size || bytes.isEmpty()) break
            }
        }
        if (target.exists()) target.delete()
        if (!part.renameTo(target)) throw RequestFailed("Couldn't keep a copy of the file on the phone.")
        return target
    }

    private suspend fun piece(source: FileSource, offset: Long): Triple<Long, Long, String> =
        when (source) {
            is FileSource.Folder -> {
                val r = connection.request(Request.FolderRead(source.session, source.path, offset)) as? Reply.FolderPart
                    ?: throw RequestFailed("Thrall sent something else instead of the file.")
                Triple(r.offset, r.size, r.data)
            }
            is FileSource.Upload -> {
                val r = connection.request(Request.FileGet(source.file, offset)) as? Reply.FilePart
                    ?: throw RequestFailed("Thrall sent something else instead of the file.")
                Triple(r.offset, r.size, r.data)
            }
        }

    /** Where a source's copy lives: one folder per session or upload, the file's own name inside. */
    fun target(source: FileSource): File = when (source) {
        is FileSource.Folder -> File(File(cache, FOLDERS_DIR + "/" + safe(source.session)), safePath(source.path))
        is FileSource.Upload -> File(File(cache, UPLOADS_DIR + "/" + safe(source.file)), safe(source.name))
    }

    companion object {
        const val FOLDERS_DIR = "files/sessions"
        const val UPLOADS_DIR = "files/uploads"
        private const val PART_SUFFIX = ".part"

        /** One path part, safe as a file name on the phone. */
        fun safe(part: String): String = part.map { if (it.isLetterOrDigit() || it in "._- ") it else '_' }
            .joinToString("").trim().trimStart('.').ifEmpty { "file" }

        /** A relative path, each part made safe; nothing can climb out with `..`. */
        fun safePath(path: String): String =
            path.split('/', '\\').filter { it.isNotBlank() && it != "." && it != ".." }.joinToString("/") { safe(it) }
                .ifEmpty { "file" }
    }
}
