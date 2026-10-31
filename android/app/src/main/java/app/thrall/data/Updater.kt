package app.thrall.data

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import app.thrall.BuildConfig
import app.thrall.protocol.AppRelease
import app.thrall.protocol.Reply
import app.thrall.protocol.Request
import java.io.File
import java.io.OutputStream
import java.security.MessageDigest
import java.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

/** Where an update to this app stands. */
sealed interface UpdateState {
    /** Nothing newer on offer (or not asked yet). */
    data object None : UpdateState
    data class Available(val release: AppRelease) : UpdateState
    /** Android needs "Install unknown apps" switched on for Thrall first. */
    data class NeedsPermission(val release: AppRelease) : UpdateState
    data class Downloading(val release: AppRelease, val got: Long) : UpdateState
    /** Handed to Android, which asks the person to confirm. */
    data class Installing(val release: AppRelease) : UpdateState
    data class Failed(val release: AppRelease, val reason: String) : UpdateState
}

/**
 * Updates the app from the computer: the engine offers the newest build (`app.latest`), the phone
 * downloads it over the encrypted link (`app.get`), checks it against the SHA-256 the engine gave,
 * and hands it to Android's installer. Android asks the person to confirm unless Thrall installed
 * the version being replaced itself; it also refuses anything not signed with the same key.
 */
class Updater(private val context: Context, private val connection: Connection) {
    private val _state = MutableStateFlow<UpdateState>(UpdateState.None)
    val state: StateFlow<UpdateState> = _state

    /**
     * Ask the engine what it offers (an engine too old to know `app.latest` offers nothing).
     * Returns the new state; throws when the engine couldn't be asked, so a button can say so.
     */
    suspend fun check(): UpdateState {
        val release = (connection.request(Request.AppLatest) as? Reply.AppLatest)?.release
        val busy = _state.value is UpdateState.Downloading || _state.value is UpdateState.Installing
        if (busy) return _state.value
        _state.value = if (release != null && release.versionCode > BuildConfig.VERSION_CODE) {
            UpdateState.Available(release)
        } else {
            UpdateState.None
        }
        return _state.value
    }

    /** Whether Android lets Thrall install apps (the person allows it once in Settings). */
    fun allowed(): Boolean = context.packageManager.canRequestPackageInstalls()

    /** The Settings page where the person allows that. */
    fun permissionIntent(): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))

    /** Download [release], check it, and hand it to Android. */
    suspend fun install(release: AppRelease) {
        if (!allowed()) {
            _state.value = UpdateState.NeedsPermission(release)
            return
        }
        _state.value = UpdateState.Downloading(release, 0)
        try {
            val apk = File(File(context.cacheDir, UPDATE_DIR).apply { mkdirs() }, APK_NAME)
            withContext(Dispatchers.IO) {
                apk.outputStream().use { out ->
                    fetch(release, out, piece = { offset ->
                        val r = connection.request(Request.AppGet(offset)) as? Reply.FilePart
                            ?: throw RequestFailed("Thrall sent something else instead of the update.")
                        Triple(r.offset, r.size, r.data)
                    }) { got -> _state.value = UpdateState.Downloading(release, got) }
                }
                hand(apk)
            }
            _state.value = UpdateState.Installing(release)
        } catch (e: Exception) {
            _state.value = UpdateState.Failed(release, e.message ?: "The update didn't download.")
        }
    }

    /** Android's answer, from [UpdateReceiver]. A successful install restarts the app, so only failures arrive. */
    fun finished(message: String?) {
        val release = when (val s = _state.value) {
            is UpdateState.Installing -> s.release
            is UpdateState.Downloading -> s.release
            else -> return
        }
        _state.value = UpdateState.Failed(release, message ?: "Android didn't install the update.")
    }

    private fun hand(apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            // Updates after the first one Thrall installed itself can go in without a prompt.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite(APK_NAME, 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val intent = Intent(context, UpdateReceiver::class.java)
            // Mutable: the installer adds its status to the intent.
            val pending = PendingIntent.getBroadcast(context, id, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
            session.commit(pending.intentSender)
        }
        apk.delete()
    }

    companion object {
        private const val UPDATE_DIR = "update"
        private const val APK_NAME = "Thrall.apk"

        /**
         * Copy a release into [out] a piece at a time ([piece] gives (start, total size, base64) from
         * an offset), then check its size and SHA-256 against [release]. Throws if anything is off.
         */
        suspend fun fetch(
            release: AppRelease,
            out: OutputStream,
            piece: suspend (Long) -> Triple<Long, Long, String>,
            progress: (Long) -> Unit = {},
        ) {
            val digest = MessageDigest.getInstance("SHA-256")
            var offset = 0L
            while (true) {
                val (start, size, data) = piece(offset)
                if (start != offset || size != release.size) throw RequestFailed("The update changed while it was downloading. Try again.")
                val bytes = Base64.getDecoder().decode(data)
                out.write(bytes)
                digest.update(bytes)
                offset += bytes.size
                progress(offset)
                if (offset >= size || bytes.isEmpty()) break
            }
            val sha = digest.digest().joinToString("") { "%02x".format(it) }
            if (offset != release.size || !sha.equals(release.sha256, ignoreCase = true)) {
                throw RequestFailed("The update didn't arrive whole. Try again.")
            }
        }
    }
}
