package app.thrall.data

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.FileProvider
import java.io.File
import java.io.InputStream
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** A file picked on the phone (a photo, a document) to send to Thrall. */
data class Picked(val name: String, val size: Long?, val image: Boolean)

/** Reads files the person picks on the phone, and makes room for photos taken with the camera. */
class PhoneFiles(private val context: Context) {
    private val resolver: ContentResolver get() = context.contentResolver

    /** The picked file's name, size (when the phone knows it) and whether it's a picture. */
    fun info(uri: Uri): Picked {
        var name: String? = null
        var size: Long? = null
        runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 && !c.isNull(it) }?.let { name = c.getString(it) }
                    c.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !c.isNull(it) }?.let { size = c.getLong(it) }
                }
            }
        }
        val shownName = name ?: uri.lastPathSegment?.substringAfterLast('/') ?: FALLBACK_NAME
        val type = resolver.getType(uri).orEmpty()
        val image = type.startsWith("image/") || FileKinds.kind(shownName) == FileKind.Image
        return Picked(shownName, size, image)
    }

    fun open(uri: Uri): InputStream =
        resolver.openInputStream(uri) ?: throw RequestFailed("Couldn't read that file on the phone.")

    /** A fresh place for the camera to save a photo, as a link the camera app may write to. */
    /** The app's own link to a file in its cache (one fetched from the computer), to attach it again. */
    fun uriOf(file: File): Uri = FileProvider.getUriForFile(context, PROVIDER, file)

    fun newPhoto(): Uri {
        val folder = File(context.cacheDir, CAMERA_DIR).apply { mkdirs() }
        val stamp = SimpleDateFormat(PHOTO_STAMP, Locale.US).format(Date())
        return FileProvider.getUriForFile(context, PROVIDER, File(folder, "photo-$stamp.jpg"))
    }

    companion object {
        /** The app's file provider (AndroidManifest.xml, res/xml/file_paths.xml). */
        const val PROVIDER = "app.thrall.files"
        /** Photos taken for a message, in the app's cache. */
        const val CAMERA_DIR = "camera"
        private const val PHOTO_STAMP = "yyyyMMdd-HHmmss"
        private const val FALLBACK_NAME = "file"
    }
}
