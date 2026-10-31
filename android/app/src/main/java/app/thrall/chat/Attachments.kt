package app.thrall.chat

/** A file picked for the next message: uploading, sent to the computer ([file] set), or failed. */
data class Attachment(
    val key: Long,
    val name: String,
    val image: Boolean,
    /** Bytes, when the phone knows. */
    val size: Long?,
    /** Bytes uploaded so far. */
    val sent: Long = 0,
    /** The upload's id once it's all on the computer. */
    val file: String? = null,
    /** Why it couldn't be sent; null while uploading or once done. */
    val problem: String? = null,
    /** Too big to send at all: trying again won't help. */
    val tooBig: Boolean = false,
) {
    val ready: Boolean get() = file != null
    val uploading: Boolean get() = file == null && problem == null

    /** How far the upload is, 0..1 (0 while the size is unknown). */
    val progress: Float get() = size?.takeIf { it > 0 }?.let { (sent.toFloat() / it).coerceIn(0f, 1f) } ?: 0f
}

/** The rules for sending a message with files. */
object Attachments {
    /** Most files on one message. */
    const val MAX_FILES = 10

    /** Sent when there are files but no words (the engine needs text; the web app says the same). */
    const val FILES_ONLY_ONE = "Please look at the attached file."
    const val FILES_ONLY_MANY = "Please look at the attached files."

    /**
     * Whether Send works: some text or some files, every file finished uploading, none failed, and
     * (with files) Thrall not in the middle of something, since the engine only takes files then.
     */
    fun canSend(text: String, files: List<Attachment>, working: Boolean): Boolean {
        if (files.isEmpty()) return text.isNotBlank()
        return !working && files.all { it.ready }
    }

    /** The message's text: what was typed, or a short request when only files were picked. */
    fun text(typed: String, files: List<Attachment>): String = when {
        typed.isNotBlank() -> typed.trim()
        files.size == 1 -> FILES_ONLY_ONE
        files.isNotEmpty() -> FILES_ONLY_MANY
        else -> ""
    }

    /** How many more files fit on the message. */
    fun room(files: List<Attachment>): Int = (MAX_FILES - files.size).coerceAtLeast(0)
}
