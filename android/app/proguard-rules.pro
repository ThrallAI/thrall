# kotlinx.serialization keeps what it needs through its own rules; BouncyCastle is used through
# its lightweight API directly (no provider lookups by name), so R8 may shrink it.
-dontwarn org.bouncycastle.**
