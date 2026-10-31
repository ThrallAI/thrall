package app.thrall.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** What the phone remembers about the engine it's paired with. */
data class Pairing(
    /** The engine's public address (`https://…`). */
    val address: String,
    /** The engine's public key: only that engine can answer the handshake. */
    val engineKey: ByteArray,
    /** This phone's id on the engine. */
    val phoneId: String,
    /** This phone's private key (kept encrypted at rest; see [PairingStore]). */
    val phonePrivate: ByteArray,
)

/**
 * The pairing, in the app's private storage. The phone's private key is encrypted with an AES key
 * that lives in the Android Keystore and never leaves it, so a copy of the app's files alone can't
 * pose as this phone. Nothing here is backed up (`allowBackup="false"`).
 */
class PairingStore(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun load(): Pairing? {
        val address = prefs.getString(ADDRESS, null) ?: return null
        val engineKey = prefs.getString(ENGINE_KEY, null)?.let(::unb64) ?: return null
        val phoneId = prefs.getString(PHONE_ID, null) ?: return null
        val sealed = prefs.getString(PHONE_KEY, null)?.let(::unb64) ?: return null
        val private = try {
            unwrap(sealed)
        } catch (e: Exception) {
            // The Keystore key is gone (e.g. the phone was reset): pair again.
            return null
        }
        return Pairing(address, engineKey, phoneId, private)
    }

    fun save(pairing: Pairing) {
        prefs.edit()
            .putString(ADDRESS, pairing.address)
            .putString(ENGINE_KEY, b64(pairing.engineKey))
            .putString(PHONE_ID, pairing.phoneId)
            .putString(PHONE_KEY, b64(wrap(pairing.phonePrivate)))
            .apply()
    }

    fun forget() {
        prefs.edit().clear().apply()
        runCatching { keystore().deleteEntry(ALIAS) }
    }

    private fun keystore() = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    private fun wrappingKey(): SecretKey {
        (keystore().getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private fun wrap(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.ENCRYPT_MODE, wrappingKey())
        return cipher.iv + cipher.doFinal(plain)
    }

    private fun unwrap(sealed: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.DECRYPT_MODE, wrappingKey(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_LEN))
        return cipher.doFinal(sealed, IV_LEN, sealed.size - IV_LEN)
    }

    private companion object {
        const val PREFS = "pairing"
        const val ADDRESS = "address"
        const val ENGINE_KEY = "engine_key"
        const val PHONE_ID = "phone_id"
        const val PHONE_KEY = "phone_key"
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "thrall-phone-key"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
        const val TAG_BITS = 128

        fun b64(bytes: ByteArray): String = Base64.getEncoder().encodeToString(bytes)
        fun unb64(text: String): ByteArray? = runCatching { Base64.getDecoder().decode(text) }.getOrNull()
    }
}
