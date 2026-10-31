package app.thrall.ui

import android.content.ContentValues
import android.content.Context
import android.os.Environment
import android.provider.MediaStore
import android.widget.Toast
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalContext
import app.thrall.data.Downloads
import app.thrall.data.FileSource
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * What can be done with a file in a conversation, by holding it down: see it in the session's
 * folder, share it to another app, open it with one, save a copy in the phone's Downloads, or send
 * it to another Thrall chat.
 */
@Composable
fun FileMenu(source: FileSource, files: FileActions, expanded: Boolean, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val withCopy: (suspend (File) -> Unit) -> Unit = { use ->
        onDismiss()
        scope.launch {
            try {
                val cached = files.downloads.target(source)
                use(if (cached.exists()) cached else files.downloads.fetch(source))
            } catch (e: Exception) {
                Toast.makeText(context, e.message ?: "Couldn't get ${source.name}.", Toast.LENGTH_LONG).show()
            }
        }
    }
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        if (source is FileSource.Folder && files.showIn != null) {
            DropdownMenuItem(text = { Text("Show in Files") }, onClick = { onDismiss(); files.showIn.invoke(source.path) })
        }
        DropdownMenuItem(text = { Text("Share…") }, onClick = { withCopy { share(context, it, source.name) } })
        DropdownMenuItem(text = { Text("Open with…") }, onClick = {
            withCopy { f -> openWith(context, f, source.name)?.let { Toast.makeText(context, it, Toast.LENGTH_LONG).show() } }
        })
        DropdownMenuItem(text = { Text("Save to phone") }, onClick = {
            withCopy { f ->
                val note = withContext(Dispatchers.IO) { saveToPhone(context, f, source.name) }
                Toast.makeText(context, note, Toast.LENGTH_LONG).show()
            }
        })
        if (files.forward != null) {
            DropdownMenuItem(text = { Text("Send to another chat…") }, onClick = { withCopy { files.forward.invoke(it, source.name) } })
        }
    }
}

/** A copy of [file] in the phone's Downloads\Thrall folder; says where, or why not. */
fun saveToPhone(context: Context, file: File, name: String): String {
    val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, Downloads.safe(name))
        put(MediaStore.Downloads.MIME_TYPE, mimeOf(name))
        put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$SAVE_FOLDER")
    }
    val resolver = context.contentResolver
    val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        ?: return "Couldn't save $name to the phone."
    return try {
        resolver.openOutputStream(uri).use { out -> file.inputStream().use { it.copyTo(out ?: error("no output")) } }
        "Saved to Downloads/$SAVE_FOLDER/$name"
    } catch (e: Exception) {
        resolver.delete(uri, null, null)
        "Couldn't save $name to the phone: ${e.message}"
    }
}

/** The folder in the phone's Downloads that saved files go in. */
private const val SAVE_FOLDER = "Thrall"
