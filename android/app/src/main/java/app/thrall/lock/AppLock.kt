package app.thrall.lock

import android.app.Activity
import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators.BIOMETRIC_WEAK
import android.hardware.biometrics.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import android.os.SystemClock
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Locks the app behind the phone's own fingerprint, face or screen lock. Off until the person
 * switches it on; then the app is locked when it starts and after [LOCK_AFTER_MS] away. While
 * it's on, the recent-apps view doesn't show the chat (Android 13 and later).
 */
class AppLock(private val context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    var enabled: Boolean
        get() = prefs.getBoolean(ENABLED, false)
        private set(value) = prefs.edit().putBoolean(ENABLED, value).apply()

    /** Whether the app is locked, and why the last unlock didn't work. */
    data class Locked(val locked: Boolean, val problem: String?)

    private val _lockedState = MutableStateFlow(Locked(enabled, null))
    val locked: StateFlow<Locked> = _lockedState

    private var awayAt: Long? = null

    /** The phone has a fingerprint, face or screen lock the app can use. */
    fun available(): Boolean =
        context.getSystemService(BiometricManager::class.java)
            ?.canAuthenticate(BIOMETRIC_WEAK or DEVICE_CREDENTIAL) == BiometricManager.BIOMETRIC_SUCCESS

    /** The app went out of sight. */
    fun away() {
        awayAt = SystemClock.elapsedRealtime()
    }

    /** The app came back: lock it if it was gone long enough. */
    fun back(activity: Activity) {
        hideFromRecents(activity)
        val since = awayAt ?: return
        awayAt = null
        if (enabled && SystemClock.elapsedRealtime() - since >= LOCK_AFTER_MS) {
            _lockedState.value = Locked(true, null)
        }
    }

    /** Ask for the fingerprint, face or screen lock, and unlock on success. */
    fun unlock(activity: Activity) = prompt(activity, "Unlock Thrall") { _lockedState.value = Locked(false, null) }

    /**
     * Switch the lock on or off; either way the person proves they can unlock first, so the app
     * can't be locked by someone who can't open it again (or unlocked by someone who isn't them).
     */
    fun setEnabled(activity: Activity, on: Boolean, done: (String?) -> Unit) {
        if (on && !available()) {
            done("Set up a fingerprint, face or screen lock on the phone first.")
            return
        }
        prompt(activity, if (on) "Turn on app lock" else "Turn off app lock", failed = done) {
            enabled = on
            hideFromRecents(activity)
            done(null)
        }
    }

    /**
     * Prove it's the phone's owner (fingerprint, face or screen lock) before something risky, such
     * as switching on Full auto. [done] gets null when they did, or why not; nothing happens when
     * the phone has no lock to check, because then anyone holding it would pass.
     */
    fun confirm(activity: Activity, title: String, done: (String?) -> Unit) {
        if (!available()) {
            done("Set up a fingerprint, face or screen lock on the phone first.")
            return
        }
        prompt(activity, title, failed = { why -> done(why ?: CANCELLED) }) { done(null) }
    }

    private fun hideFromRecents(activity: Activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) activity.setRecentsScreenshotEnabled(!enabled)
    }

    private fun prompt(activity: Activity, title: String, failed: (String?) -> Unit = {}, ok: () -> Unit) {
        val prompt = BiometricPrompt.Builder(activity)
            .setTitle(title)
            .setAllowedAuthenticators(BIOMETRIC_WEAK or DEVICE_CREDENTIAL)
            .build()
        prompt.authenticate(
            CancellationSignal(),
            activity.mainExecutor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult?) = ok()

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence?) {
                    // Closing the prompt isn't worth a message; anything else is.
                    val cancelled = errorCode == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                        errorCode == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                    val why = if (cancelled) null else errString?.toString()
                    if (_lockedState.value.locked) _lockedState.value = Locked(true, why)
                    failed(why)
                }
            },
        )
    }

    companion object {
        /** Away this long and the app locks again. */
        const val LOCK_AFTER_MS = 60_000L
        /** [confirm]'s answer when the person closed the prompt. */
        const val CANCELLED = "Cancelled."
        private const val PREFS = "lock"
        private const val ENABLED = "enabled"
    }
}
