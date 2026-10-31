package app.thrall.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** Thrall's design tokens (web/src/styles/tokens.css), dark first, light following the phone. */
@Immutable
data class Tokens(
    val bg: Color,
    val surface: Color,
    val raised: Color,
    val line: Color,
    val ink: Color,
    val inkMuted: Color,
    val accent: Color,
    val accentFill: Color,
    val accentSoft: Color,
    val code: Color,
    val ok: Color,
    val warn: Color,
    val danger: Color,
)

val DarkTokens = Tokens(
    bg = Color(0xFF111018), surface = Color(0xFF17161F), raised = Color(0xFF201E2B), line = Color(0xFF2C2A38),
    ink = Color(0xFFECEBF2), inkMuted = Color(0xFF9D9AAE), accent = Color(0xFF8B6CFF), accentFill = Color(0xFF7457F0),
    accentSoft = Color(0xFF2A2340), code = Color(0xFFD6AAFF), ok = Color(0xFF4ADE80), warn = Color(0xFFFACC15),
    danger = Color(0xFFFF6B6B),
)

val LightTokens = Tokens(
    bg = Color(0xFFFAFAFC), surface = Color(0xFFF2F1F6), raised = Color(0xFFFFFFFF), line = Color(0xFFE2E0EA),
    ink = Color(0xFF18171F), inkMuted = Color(0xFF5F5C70), accent = Color(0xFF6A45F0), accentFill = Color(0xFF6A45F0),
    accentSoft = Color(0xFFECE7FF), code = Color(0xFF7A3DB8), ok = Color(0xFF15803D), warn = Color(0xFF8A5300),
    danger = Color(0xFFC42B2B),
)

val LocalTokens = staticCompositionLocalOf { DarkTokens }

@Composable
fun ThrallTheme(content: @Composable () -> Unit) {
    val t = if (isSystemInDarkTheme()) DarkTokens else LightTokens
    val scheme = if (t == DarkTokens) {
        darkColorScheme(
            primary = t.accentFill, onPrimary = Color.White, background = t.bg, onBackground = t.ink,
            surface = t.surface, onSurface = t.ink, surfaceVariant = t.raised, onSurfaceVariant = t.inkMuted,
            outline = t.line, error = t.danger, primaryContainer = t.accentSoft, onPrimaryContainer = t.ink,
        )
    } else {
        lightColorScheme(
            primary = t.accentFill, onPrimary = Color.White, background = t.bg, onBackground = t.ink,
            surface = t.surface, onSurface = t.ink, surfaceVariant = t.raised, onSurfaceVariant = t.inkMuted,
            outline = t.line, error = t.danger, primaryContainer = t.accentSoft, onPrimaryContainer = t.ink,
        )
    }
    CompositionLocalProvider(LocalTokens provides t) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
