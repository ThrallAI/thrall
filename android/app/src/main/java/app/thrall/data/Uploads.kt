package app.thrall.data

import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import java.io.InputStream
import java.util.Base64

/**
 * Sends files from the phone to the computer over the encrypted link, a piece at a time
 * (`file.put`). [put] sends one piece (to a new upload when its id is null) and returns the
 * upload's id; the app's comes from [Connection], tests give their own.
 */
class Uploads(private val put: suspend (upload: String?, name: String, data: String) -> String) {
    constructor(connection: Connection) : this({ upload, name, data ->
        (connection.request(Request.FilePut(upload, name, data)) as? Reply.FileSaved)?.file
            ?: throw RequestFailed("Thrall didn't take the file.")
    })

    /**
     * Send all of [input] as [name]; [progress] gets the bytes sent so far. Returns the upload's id
     * for `turn.send`. A file over [MAX_BYTES] stops with [RequestFailed] before more is sent.
     */
    suspend fun send(name: String, input: InputStream, progress: (Long) -> Unit = {}): String {
        var upload: String? = null
        var sent = 0L
        val buffer = ByteArray(PIECE_BYTES)
        while (true) {
            val n = fill(input, buffer)
            if (n == 0 && upload != null) break
            sent += n
            if (sent > MAX_BYTES) throw RequestFailed(tooBig(name))
            val id = put(upload, name, Base64.getEncoder().encodeToString(buffer.copyOf(n)))
            if (upload == null) upload = id
            progress(sent)
            if (n < PIECE_BYTES) break
        }
        return upload ?: throw RequestFailed("Thrall didn't take the file.")
    }

    /** Read until [buffer] is full or the file ends; how many bytes were read. */
    private fun fill(input: InputStream, buffer: ByteArray): Int {
        var total = 0
        while (total < buffer.size) {
            val n = input.read(buffer, total, buffer.size - total)
            if (n < 0) break
            total += n
        }
        return total
    }

    companion object {
        /** Bytes in one piece: the engine takes at most 512 KiB before encoding. */
        const val PIECE_BYTES = 512 * 1024

        /** Largest file the engine takes. */
        const val MAX_BYTES = 25L * 1024 * 1024

        fun tooBig(name: String) = "$name is over ${MAX_BYTES / (1024 * 1024)} MB, the most Thrall takes."
    }
}
