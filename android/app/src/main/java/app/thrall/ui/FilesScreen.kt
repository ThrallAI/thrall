package app.thrall.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import android.text.format.Formatter
import android.webkit.MimeTypeMap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import app.thrall.data.Connection
import app.thrall.data.Downloads
import app.thrall.data.FileKind
import app.thrall.data.FileKinds
import app.thrall.data.FileSource
import app.thrall.data.RequestFailed
import app.thrall.protocol.FolderEntry
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The engine on the computer is older than this feature. */
private const val NEEDS_NEWER_ENGINE = "Update Thrall on the computer to browse files from the phone."

/** The file provider's authority (AndroidManifest.xml). */
private const val PROVIDER = app.thrall.data.PhoneFiles.PROVIDER

/** Largest picture decoded at full size; bigger ones are scaled down to fit memory. */
private const val IMAGE_MAX_PX = 4096

/** Browse a session's folder; [onOpen] gets a file's path inside it. */
@Composable
fun FolderScreen(connection: Connection, session: String, title: String, path: String, onOpen: (String) -> Unit, onFolder: (String) -> Unit, onBack: () -> Unit) {
    val t = LocalTokens.current
    var items by remember(path) { mutableStateOf<List<FolderEntry>?>(null) }
    var more by remember(path) { mutableStateOf(false) }
    var problem by remember(path) { mutableStateOf<String?>(null) }
    var reload by remember { mutableStateOf(0) }
    LaunchedEffect(session, path, reload) {
        problem = null
        try {
            val r = connection.request(Request.FolderList(session, path)) as? Reply.FolderListed
            items = r?.items.orEmpty()
            more = r?.more == true
        } catch (e: RequestFailed) {
            problem = if (e.code == "unsupported") NEEDS_NEWER_ENGINE else e.message
        } catch (e: Exception) {
            problem = e.message
        }
    }
    Column(Modifier.fillMaxSize()) {
        Header(
            title = if (path.isEmpty()) "Files" else path.substringAfterLast('/'),
            subtitle = if (path.isEmpty()) title else path,
            onBack = onBack,
        ) { TextButton(onClick = { reload++ }) { Text("Refresh", color = t.accent) } }
        when {
            problem != null -> Text(problem.orEmpty(), color = t.danger, fontSize = 14.sp, modifier = Modifier.padding(16.dp))
            items == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            items.orEmpty().isEmpty() -> Text("Nothing here yet.", color = t.inkMuted, modifier = Modifier.padding(16.dp))
            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(items.orEmpty(), key = { it.name }) { e ->
                    val inner = if (path.isEmpty()) e.name else "$path/${e.name}"
                    EntryRow(e) { if (e.folder) onFolder(inner) else onOpen(inner) }
                    HorizontalDivider(color = t.line, modifier = Modifier.padding(start = 16.dp))
                }
                if (more) item { Text("Some files aren't shown (a very full folder).", color = t.inkMuted, fontSize = 12.sp, modifier = Modifier.padding(16.dp)) }
            }
        }
    }
}

@Composable
private fun EntryRow(e: FolderEntry, onClick: () -> Unit) {
    val t = LocalTokens.current
    val context = LocalContext.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(if (e.folder) "▸" else iconFor(e.name), color = if (e.folder) t.accent else t.inkMuted, modifier = Modifier.padding(end = 12.dp))
        Column(Modifier.weight(1f)) {
            Text(e.name, color = t.ink, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val ago = DateUtils.getRelativeTimeSpanString(e.modified, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS)
            val detail = if (e.folder) ago.toString() else "${Formatter.formatShortFileSize(context, e.size)} · $ago"
            Text(detail, color = t.inkMuted, fontSize = 12.sp)
        }
    }
}

private fun iconFor(name: String) = when (FileKinds.kind(name)) {
    FileKind.Image -> "▣"
    FileKind.Markdown, FileKind.Text -> "≡"
    FileKind.Other -> "◇"
}

/** One file: fetched from the computer, then shown here or handed to another app. */
@Composable
fun FileScreen(downloads: Downloads, source: FileSource, onBack: () -> Unit) {
    val t = LocalTokens.current
    val context = LocalContext.current
    var copy by remember(source) { mutableStateOf<File?>(null) }
    var progress by remember(source) { mutableFloatStateOf(0f) }
    var problem by remember(source) { mutableStateOf<String?>(null) }
    LaunchedEffect(source) {
        try {
            copy = downloads.fetch(source) { done, total -> progress = if (total > 0) done.toFloat() / total else 1f }
        } catch (e: RequestFailed) {
            problem = if (e.code == "unsupported") NEEDS_NEWER_ENGINE else e.message
        } catch (e: Exception) {
            problem = e.message ?: "Couldn't fetch the file."
        }
    }
    val file = copy
    Column(Modifier.fillMaxSize()) {
        Header(title = source.name, subtitle = (source as? FileSource.Folder)?.path, onBack = onBack) {
            if (file != null) TextButton(onClick = { share(context, file, source.name) }) { Text("Share", color = t.accent) }
        }
        when {
            problem != null -> Text(problem.orEmpty(), color = t.danger, modifier = Modifier.padding(16.dp))
            file == null -> Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                CircularProgressIndicator()
                Text("Fetching… ${(progress * 100).toInt()}%", color = t.inkMuted, modifier = Modifier.padding(top = 12.dp))
            }
            else -> when (FileKinds.kind(source.name, file.length())) {
                FileKind.Image -> ZoomImage(file)
                FileKind.Markdown -> TextView(file, markdown = true)
                FileKind.Text -> TextView(file, markdown = false)
                FileKind.Other -> Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(source.name, color = t.ink, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                    Text(Formatter.formatShortFileSize(context, file.length()), color = t.inkMuted)
                    Text("This kind of file opens in another app on your phone.", color = t.inkMuted, fontSize = 14.sp)
                    Button(onClick = { problem = openWith(context, file, source.name) }) { Text("Open with…") }
                    OutlinedButton(onClick = { share(context, file, source.name) }) { Text("Share") }
                }
            }
        }
    }
}

@Composable
private fun TextView(file: File, markdown: Boolean) {
    val t = LocalTokens.current
    var text by remember(file) { mutableStateOf<String?>(null) }
    LaunchedEffect(file) { text = withContext(Dispatchers.IO) { file.readText() } }
    val body = text ?: return
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        if (markdown) {
            Markdown(body)
        } else {
            SelectionContainer {
                Text(
                    body,
                    color = t.ink,
                    style = TextStyle(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 17.sp),
                    softWrap = false,
                    modifier = Modifier.horizontalScroll(rememberScrollState()),
                )
            }
        }
    }
}

/** A picture filling the screen: pinch to zoom, drag to move. */
@Composable
private fun ZoomImage(file: File) {
    val t = LocalTokens.current
    var bitmap by remember(file) { mutableStateOf<ImageBitmap?>(null) }
    var failed by remember(file) { mutableStateOf(false) }
    LaunchedEffect(file) {
        bitmap = withContext(Dispatchers.IO) { decode(file) }
        failed = bitmap == null
    }
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    val image = bitmap
    Box(
        Modifier
            .fillMaxSize()
            .background(androidx.compose.ui.graphics.Color.Black)
            .pointerInput(Unit) {
                detectTransformGestures { _, pan, zoom, _ ->
                    scale = (scale * zoom).coerceIn(1f, 8f)
                    offset = if (scale == 1f) Offset.Zero else offset + pan
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        when {
            image != null -> Image(
                bitmap = image,
                contentDescription = file.name,
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize().graphicsLayer(
                    scaleX = scale,
                    scaleY = scale,
                    translationX = offset.x,
                    translationY = offset.y,
                ),
            )
            failed -> Text("This picture couldn't be shown.", color = t.inkMuted)
            else -> CircularProgressIndicator()
        }
    }
}

/** A picture from disk, scaled down if it's very large. Null if it isn't one. */
fun decode(file: File, maxPx: Int = IMAGE_MAX_PX): ImageBitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    if (bounds.outWidth <= 0) return null
    var sample = 1
    while (bounds.outWidth / sample > maxPx || bounds.outHeight / sample > maxPx) sample *= 2
    return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
}

internal fun mimeOf(name: String): String =
    FileKinds.mime(name)
        ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(FileKinds.extension(name))
        ?: "application/octet-stream"

private fun uriOf(context: Context, file: File) = FileProvider.getUriForFile(context, PROVIDER, file)

/** Open in another app; a reason if none can. */
internal fun openWith(context: Context, file: File, name: String): String? {
    val intent = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uriOf(context, file), mimeOf(name))
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    return try {
        context.startActivity(Intent.createChooser(intent, "Open $name with").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        null
    } catch (e: ActivityNotFoundException) {
        "No app on this phone opens .${FileKinds.extension(name)} files. Share it to one that can, or open it on the computer."
    }
}

internal fun share(context: Context, file: File, name: String) {
    val intent = Intent(Intent.ACTION_SEND).apply {
        type = mimeOf(name)
        putExtra(Intent.EXTRA_STREAM, uriOf(context, file))
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    context.startActivity(Intent.createChooser(intent, "Share $name").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/**
 * A screen's top row: Back, a title (and a smaller line under it, tappable with [onSubtitle]), and
 * room for actions.
 */
@Composable
fun Header(
    title: String,
    subtitle: String?,
    onBack: () -> Unit,
    onSubtitle: (() -> Unit)? = null,
    actions: @Composable () -> Unit = {},
) {
    val t = LocalTokens.current
    Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
        TextButton(onClick = onBack) { Text("‹ Back", color = t.accent) }
        Column(Modifier.weight(1f)) {
            Text(title, color = t.ink, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!subtitle.isNullOrEmpty()) {
                Text(
                    subtitle,
                    color = if (onSubtitle != null) t.accent else t.inkMuted,
                    fontSize = 11.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = if (onSubtitle != null) Modifier.clickable(onClick = onSubtitle) else Modifier,
                )
            }
        }
        actions()
    }
}
