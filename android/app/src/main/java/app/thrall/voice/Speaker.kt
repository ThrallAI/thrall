package app.thrall.voice

import android.content.Context
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.widget.Toast
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Reads Thrall's replies aloud with the phone's own text-to-speech (the voice and language set in
 * Android's settings). Free, on the phone, nothing sent anywhere.
 */
class Speaker(context: Context) {
    private val context = context.applicationContext
    private val _speaking = MutableStateFlow<String?>(null)

    /** The key of the reply being read, or null. */
    val speaking: StateFlow<String?> = _speaking

    private var ready = false
    private var failed = false
    private var waiting: (() -> Unit)? = null
    private var lastPiece: String? = null
    private var pieces = 0
    private val tts: TextToSpeech = TextToSpeech(this.context) { status ->
        ready = status == TextToSpeech.SUCCESS
        failed = !ready
        if (ready) waiting?.invoke() else if (waiting != null) noVoice()
        waiting = null
    }

    init {
        tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(id: String) = Unit
            override fun onDone(id: String) = finished(id)
            @Deprecated("Deprecated in Java")
            override fun onError(id: String) = finished(id)
            override fun onError(id: String, errorCode: Int) = finished(id)
        })
    }

    private fun finished(id: String) {
        if (id == lastPiece) _speaking.value = null
    }

    /** Read [markdown] aloud as [key]: straight away, or after what's being read when [after]. */
    fun speak(key: String, markdown: String, after: Boolean = false) {
        val text = SpeechText.spoken(markdown)
        if (text.isBlank()) return
        if (failed) return noVoice()
        if (!ready) {
            waiting = { speak(key, markdown, after) }
            return
        }
        val parts = SpeechText.pieces(text, TextToSpeech.getMaxSpeechInputLength())
        for ((i, part) in parts.withIndex()) {
            val id = "$key#${pieces++}"
            val mode = if (i == 0 && !after) TextToSpeech.QUEUE_FLUSH else TextToSpeech.QUEUE_ADD
            tts.speak(part, mode, null, id)
            lastPiece = id
        }
        _speaking.value = key
    }

    /** Read [key] aloud, or stop if it's the one being read. */
    fun toggle(key: String, markdown: String) = if (_speaking.value == key) stop() else speak(key, markdown)

    fun stop() {
        waiting = null
        if (ready) tts.stop()
        _speaking.value = null
    }

    fun shutdown() {
        stop()
        tts.shutdown()
    }

    private fun noVoice() {
        Toast.makeText(context, "This phone has no text-to-speech voice set up (Settings → Accessibility).", Toast.LENGTH_LONG).show()
    }
}
