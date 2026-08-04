package org.xinutec.console

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.security.keystore.UserNotAuthenticatedException
import android.util.Base64
import android.util.Log
import java.io.File
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.Signature
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import javax.security.auth.x500.X500Principal

/**
 * The key that is this phone, and the only credential the console accepts.
 *
 * It is an EC P-256 pair generated inside the device's secure element, and it never
 * leaves it: there is no file to steal, nothing a compromised server can hold, and
 * no way to copy the app's identity onto another phone. What the console pins is
 * the SHA-256 of its public key. That is the whole of the trust relationship —
 * no CA, no PKI, no name to validate.
 *
 * ## Why the key is generated here and not handed to the app
 *
 * A key the app could import would be a key that existed outside the secure element
 * at some point, and the guarantee being bought is precisely that it never did.
 * That is also what makes [enrol] worth its awkwardness: the keystore emits an
 * **attestation chain** alongside the key, a Google-rooted assertion about where the
 * key lives and under what conditions it can be used. Enrolment checks that chain on
 * the Mac (`scripts/enrol.sh` → `cargo run -p console --bin attest`) rather than
 * taking the phone's word for it.
 *
 * ## Why the digests are what they are
 *
 * ⚠ [KeyProperties.DIGEST_NONE] is not optional and its absence is invisible until
 * the first TLS handshake. Chromium signs the handshake through `NONEwithECDSA`
 * with the transcript hash already computed, so a key permitting only SHA-256
 * refuses to sign and the connection dies in `initSign` with `InvalidKeyException`
 * — from inside the network stack, where nothing says which key or why.
 *
 * ## Why one unlock covers a stretch of time
 *
 * `setUserAuthenticationRequired(true)` alone means a face scan per *signature*,
 * which is a prompt per TLS handshake. [UNLOCK_SECONDS] is the real security
 * parameter here: it is how long a snatched, unlocked phone stays useful, so it
 * belongs in the minutes. Note what it also buys — a *device* unlock inside the
 * window authorises the key, so opening the app straight after unlocking the phone
 * needs no second prompt, and [unlocked] is what asks rather than assuming.
 */
object Keys {
    /**
     * How long one authentication keeps the key usable.
     *
     * Fifteen minutes, chosen deliberately against five. The prompt is not paced
     * by use: the page polls every few seconds, so the first poll after the
     * window closes asks again whether or not anybody did anything — which made
     * a twenty-minute read cost four unlocks. This is the exposure of a phone
     * taken while unlocked, and tripling it buys a third of the interruptions.
     */
    const val UNLOCK_SECONDS = 900

    private const val ALIAS = "console-client"
    private const val TAG = "console-keys"

    /** SHA-256's output length — what a P-256 signature is taken over. */
    private const val PROBE_BYTES = 32

    /**
     * Generate a fresh key, discarding any previous one.
     *
     * [challenge] is the reason this is an operator-initiated act rather than
     * something the app does for itself: it is generated on the Mac and arrives over
     * adb, so the attestation record it appears in cannot have been prepared in
     * advance by a phone claiming to be honest.
     *
     * A device with no StrongBox falls back to the TEE rather than failing, because
     * refusing here would be refusing in the wrong place — the attestation record
     * says which one it got, and the check on the Mac is what decides whether that
     * is acceptable.
     */
    fun enrol(challenge: ByteArray) {
        store().deleteEntry(ALIAS)
        try {
            generate(challenge, strongBox = true)
        } catch (_: StrongBoxUnavailableException) {
            Log.w(
                TAG,
                "no StrongBox here — generating in the TEE; the attestation record will say so",
            )
            generate(challenge, strongBox = false)
        }
    }

    /**
     * Write the attestation chain where `adb shell run-as … cat` can reach it.
     *
     * The chain, not just the certificate: the leaf carries the attestation record
     * and the ones above it are what make the record Google's claim rather than the
     * phone's.
     */
    fun writeEnrolment(dir: File): File {
        val chain = store().getCertificateChain(ALIAS) ?: error("no key to export — enrol first")
        val file = File(dir, ENROLMENT_FILE)
        file.writeText(chain.joinToString("") { pem(it.encoded) })
        return file
    }

    /** The certificate to present, or null when there is no key yet. */
    fun leaf(): X509Certificate? =
        store().getCertificateChain(ALIAS)?.firstOrNull() as? X509Certificate

    /** The private key to sign the handshake with, or null when there is no key yet. */
    fun privateKey(): PrivateKey? = store().getKey(ALIAS, null) as? PrivateKey

    /**
     * Whether the key can be used *right now*, without asking anybody anything.
     *
     * Asked by signing something and throwing the answer away, which is what lets
     * the prompt appear before the handshake rather than during it, where there is
     * nothing to attach it to.
     *
     * ⚠ **`initSign` alone is not a test, and looks like one.** The
     * `NONEwithECDSA` implementation buffers its input and does not open a keystore
     * operation until there is something to sign, so `initSign` succeeds happily
     * against a key whose authentication window ran out ten minutes ago. Measured
     * on a locked Pixel 9: the probe said yes, the certificate was presented, and
     * the signature then failed inside the network stack — so the page failed and
     * no prompt was ever shown, which is exactly the moment the prompt exists for.
     */
    fun unlocked(): Boolean {
        val key = privateKey() ?: return false
        return try {
            val probe = Signature.getInstance("NONEwithECDSA")
            probe.initSign(key)
            // A P-256 digest's worth of zeroes: NONEwithECDSA expects an already
            // hashed input, and this one is a constant rather than anything the far
            // end chose.
            probe.update(ByteArray(PROBE_BYTES))
            probe.sign()
            true
        } catch (_: UserNotAuthenticatedException) {
            false
        } catch (_: KeyPermanentlyInvalidatedException) {
            // The device's biometric enrolment changed, which invalidates the key by
            // design. It cannot be recovered — only enrolled again, on the Mac.
            Log.e(TAG, "the key is permanently invalidated (biometrics changed) — re-enrol")
            false
        }
    }

    /** SHA-256 of a public key's SubjectPublicKeyInfo, as lowercase hex — a pin. */
    fun pin(certificate: X509Certificate): String =
        MessageDigest
            .getInstance("SHA-256")
            .digest(certificate.publicKey.encoded)
            .joinToString("") { "%02x".format(it) }

    /** The name [writeEnrolment] uses, so `enrol.sh` and this agree in one place. */
    const val ENROLMENT_FILE = "enrolment.pem"

    private fun generate(challenge: ByteArray, strongBox: Boolean) {
        val spec =
            KeyGenParameterSpec
                .Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                // DIGEST_NONE: see the class comment. Removing it breaks TLS only.
                .setDigests(KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_NONE)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(
                    UNLOCK_SECONDS,
                    KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL,
                ).setAttestationChallenge(challenge)
                .setIsStrongBoxBacked(strongBox)
                // The subject is never checked by anything — the console pins the key
                // and ignores names — so it says what it is rather than pretending to
                // be a hostname.
                .setCertificateSubject(X500Principal("CN=console client"))
                .build()
        KeyPairGenerator
            .getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
            .apply { initialize(spec) }
            .generateKeyPair()
    }

    private fun store(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun pem(der: ByteArray): String {
        val body = Base64.encodeToString(der, Base64.NO_WRAP).chunked(64).joinToString("\n")
        return "-----BEGIN CERTIFICATE-----\n$body\n-----END CERTIFICATE-----\n"
    }
}
