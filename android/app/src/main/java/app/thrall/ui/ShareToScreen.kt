package app.thrall.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.thrall.Shared
import app.thrall.protocol.SessionInfo

/** Something shared into Thrall (or sent on from a chat): pick the chat it goes to. */
@Composable
fun ShareToScreen(
    shared: Shared,
    shown: SessionsModel.Shown,
    onPick: (SessionInfo) -> Unit,
    onNew: () -> Unit,
    onCancel: () -> Unit,
) {
    val t = LocalTokens.current
    val files = shared.uris.size
    val what = listOfNotNull(
        when (files) {
            0 -> null
            1 -> "1 file"
            else -> "$files files"
        },
        shared.text.takeIf { it.isNotBlank() }?.let { "“${it.take(PREVIEW_CHARS)}${if (it.length > PREVIEW_CHARS) "…" else ""}”" },
    ).joinToString(" and ")
    Column(Modifier.fillMaxSize()) {
        Header(title = "Send to which chat?", subtitle = what.ifEmpty { null }, onBack = onCancel)
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                Text(
                    "＋ New chat",
                    color = t.accent,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier
                        .padding(horizontal = 16.dp, vertical = 8.dp)
                        .fillMaxWidth()
                        .background(t.surface, RoundedCornerShape(12.dp))
                        .clickable(onClick = onNew)
                        .padding(14.dp),
                )
            }
            shown.home?.let { home ->
                item {
                    Text(
                        "Home",
                        color = t.ink,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier
                            .padding(horizontal = 16.dp, vertical = 4.dp)
                            .fillMaxWidth()
                            .background(t.accentSoft, RoundedCornerShape(12.dp))
                            .clickable { onPick(home) }
                            .padding(14.dp),
                    )
                }
            }
            items(shown.sessions, key = { it.session }) { s ->
                Text(
                    (if (s.pinned) "📌 " else "") + s.title.ifBlank { "New session" },
                    color = t.ink,
                    fontSize = 15.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.fillMaxWidth().clickable { onPick(s) }.padding(horizontal = 16.dp, vertical = 14.dp),
                )
                HorizontalDivider(color = t.line, modifier = Modifier.padding(start = 16.dp))
            }
        }
    }
}

/** Characters of shared text shown at the top. */
private const val PREVIEW_CHARS = 60
