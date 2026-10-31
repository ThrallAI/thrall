package app.thrall.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Shown instead of everything while the app lock is closed; asks straight away. */
@Composable
fun LockScreen(problem: String?, onUnlock: () -> Unit) {
    val t = LocalTokens.current
    LaunchedEffect(Unit) { onUnlock() }
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Thrall", fontSize = 28.sp, fontWeight = FontWeight.SemiBold, color = t.ink)
        Text("Locked", color = t.inkMuted, fontSize = 15.sp)
        problem?.let { Text(it, color = t.danger, fontSize = 13.sp, textAlign = TextAlign.Center) }
        Button(onClick = onUnlock) { Text("Unlock") }
    }
}
