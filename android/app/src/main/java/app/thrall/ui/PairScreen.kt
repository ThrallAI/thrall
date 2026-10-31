package app.thrall.ui

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import app.thrall.data.Connection
import app.thrall.link.PairLink
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors
import kotlinx.coroutines.launch

/** First run (or after being removed): scan the QR from Settings → Phones on the computer. */
@Composable
fun PairScreen(connection: Connection, note: String? = null) {
    val t = LocalTokens.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var camera by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { camera = it }
    var pasted by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf(note) }

    fun pair(text: String) {
        if (busy) return
        val link = PairLink.parse(text)
        if (link == null) {
            problem = "That isn't a Thrall pairing code."
            return
        }
        busy = true
        problem = null
        scope.launch {
            val result = connection.pair(link, phoneName())
            busy = false
            result.exceptionOrNull()?.let { problem = it.message }
        }
    }

    Column(
        Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Connect to Thrall", fontSize = 26.sp, fontWeight = FontWeight.SemiBold, color = t.ink)
        Text(
            "On your computer, open Thrall → Settings → Phones, turn on remote access and choose Add a phone. Then scan the code here.",
            color = t.inkMuted,
            fontSize = 15.sp,
        )
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .clip(RoundedCornerShape(14.dp)),
            contentAlignment = Alignment.Center,
        ) {
            when {
                busy -> CircularProgressIndicator()
                camera -> QrScanner(onCode = ::pair)
                else -> Button(onClick = { ask.launch(Manifest.permission.CAMERA) }) { Text("Use the camera to scan") }
            }
        }
        problem?.let { Text(it, color = t.danger, fontSize = 14.sp) }
        Text("Or paste the pairing link", color = t.inkMuted, fontSize = 13.sp)
        OutlinedTextField(
            value = pasted,
            onValueChange = { pasted = it },
            singleLine = true,
            placeholder = { Text("thrall://pair?…") },
            // A link: no capitals or corrections from the keyboard (they'd break the code).
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Uri,
            ),
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedButton(onClick = { pair(pasted) }, enabled = pasted.isNotBlank() && !busy) { Text("Pair") }
        Text(
            "Everything between this phone and your computer is encrypted end to end. Nothing about Thrall is kept on the phone except its own key.",
            color = t.inkMuted,
            fontSize = 12.sp,
        )
    }
}

/** What the computer's phone list calls this phone. */
fun phoneName(): String {
    val maker = Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
    return if (Build.MODEL.startsWith(maker, ignoreCase = true)) Build.MODEL else "$maker ${Build.MODEL}"
}

/** The camera, reading QR codes until one is a pairing link. */
@Composable
private fun QrScanner(onCode: (String) -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val reader = remember {
        MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))) }
    }
    var found by remember { mutableStateOf(false) }
    DisposableEffect(Unit) { onDispose { executor.shutdown() } }
    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { ctx ->
            val view = PreviewView(ctx)
            val providerFuture = ProcessCameraProvider.getInstance(ctx)
            providerFuture.addListener({
                val provider = providerFuture.get()
                val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(executor) { image ->
                    val text = decode(reader, image)
                    image.close()
                    if (text != null && !found && PairLink.parse(text) != null) {
                        found = true
                        ContextCompat.getMainExecutor(context).execute { onCode(text) }
                    }
                }
                provider.unbindAll()
                provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            }, ContextCompat.getMainExecutor(ctx))
            view
        },
    )
}

private fun decode(reader: MultiFormatReader, image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
    val source = PlanarYUVLuminanceSource(bytes, plane.rowStride, image.height, 0, 0, image.width, image.height, false)
    return try {
        reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
    } catch (e: NotFoundException) {
        null
    } catch (e: Exception) {
        null
    } finally {
        reader.reset()
    }
}
