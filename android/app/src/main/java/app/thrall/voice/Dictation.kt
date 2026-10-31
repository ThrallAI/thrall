package app.thrall.voice

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import java.util.Locale

/**
 * Talking instead of typing, with the phone's own speech recognition: words come back as they're
 * heard ([onText], the whole of what's been said so far), and [onEnd] when it stops, with a reason
 * if it didn't work. Must be used on the main thread.
 */
class Dictation(
    private val context: Context,
    private val onText: (String) -> Unit,
    private val onEnd: (problem: String?) -> Unit,
) {
    private var recognizer: SpeechRecognizer? = null

    val available: Boolean get() = SpeechRecognizer.isRecognitionAvailable(context)

    fun start() {
        release()
        val r = SpeechRecognizer.createSpeechRecognizer(context)
        recognizer = r
        r.setRecognitionListener(object : RecognitionListener {
            override fun onPartialResults(partial: Bundle) = heard(partial)
            override fun onResults(results: Bundle) {
                heard(results)
                done(null)
            }
            override fun onError(error: Int) = done(problem(error))
            override fun onReadyForSpeech(params: Bundle?) = Unit
            override fun onBeginningOfSpeech() = Unit
            override fun onRmsChanged(rmsdB: Float) = Unit
            override fun onBufferReceived(buffer: ByteArray?) = Unit
            override fun onEndOfSpeech() = Unit
            override fun onEvent(eventType: Int, params: Bundle?) = Unit
        })
        r.startListening(
            Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                .putExtra(RecognizerIntent.EXTRA_LANGUAGE, Locale.getDefault().toLanguageTag())
                .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true),
        )
    }

    /** Stop listening; what was heard so far still comes back. */
    fun stop() {
        recognizer?.stopListening()
    }

    fun release() {
        recognizer?.destroy()
        recognizer = null
    }

    private fun heard(bundle: Bundle) {
        bundle.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
            ?.takeIf { it.isNotBlank() }?.let(onText)
    }

    private fun done(problem: String?) {
        release()
        onEnd(problem)
    }

    /** Why listening stopped, in words; null when it just didn't hear anything. */
    private fun problem(error: Int): String? = when (error) {
        SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT, SpeechRecognizer.ERROR_CLIENT -> null
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Thrall needs the microphone to take dictation."
        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "Speech recognition needs a network connection."
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "Speech recognition is busy; try again."
        else -> "Speech recognition didn't work (error $error)."
    }
}
