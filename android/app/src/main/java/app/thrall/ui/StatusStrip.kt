package app.thrall.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.data.LinkState
import kotlinx.coroutines.delay

/** A slim line saying whether the phone is connected to Thrall; tap it to try again now. */
@Composable
fun StatusStrip(state: LinkState, onRetry: () -> Unit) {
    val t = LocalTokens.current
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(state) {
        while (state is LinkState.Offline) {
            now = System.currentTimeMillis()
            delay(1_000)
        }
    }
    val (dot, text) = when (state) {
        is LinkState.Connected -> t.ok to "Connected" + if (state.model.isNotEmpty()) " · ${state.model}" else ""
        LinkState.Connecting -> t.warn to "Connecting…"
        is LinkState.Offline -> {
            val secs = ((state.retryAt - now) / 1000).coerceAtLeast(0)
            t.danger to "Reconnecting${if (secs > 0) " in ${secs}s" else "…"} · ${state.reason}"
        }
        is LinkState.Refused -> t.danger to state.reason
        LinkState.NotPaired -> t.inkMuted to "Not paired"
    }
    Row(
        Modifier
            .fillMaxWidth()
            .background(t.surface)
            .clickable(enabled = state is LinkState.Offline, onClick = onRetry)
            .padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(dot, CircleShape))
        Spacer(Modifier.width(8.dp))
        Text(text, color = t.inkMuted, fontSize = 12.sp, maxLines = 2, modifier = Modifier.weight(1f))
        if (state is LinkState.Offline) {
            TextButton(onClick = onRetry) { Text("Try now", color = t.accent, fontSize = 12.sp) }
        }
    }
}
