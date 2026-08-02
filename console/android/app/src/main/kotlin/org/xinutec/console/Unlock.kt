package org.xinutec.console

import android.app.Activity
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.util.Log

/**
 * Ask for a face, a fingerprint or the lock-screen PIN, so the client key becomes
 * usable again.
 *
 * The framework's [BiometricPrompt] rather than `androidx.biometric`: the AndroidX
 * one needs a `FragmentActivity` and the shell every wrapper in the fleet is built
 * on is a `ComponentActivity`. Taking the AndroidX dependency would mean dragging
 * Fragments into an app that is one WebView, to gain a compatibility surface for
 * versions this app already declines to run on.
 *
 * No `CryptoObject` is passed, and that is the point rather than an omission. A
 * crypto object binds one authentication to one operation — a prompt per signature,
 * so a prompt per TLS handshake. Authenticating without one satisfies every
 * time-bound key on the device for [Keys.UNLOCK_SECONDS], which is exactly the
 * window the key was built with.
 *
 * The device credential is allowed alongside the biometric. A face that will not
 * read in the dark must not be the only way to answer a question an agent is
 * blocked on.
 */
object Unlock {
    private const val TAG = "console-unlock"

    fun prompt(activity: Activity, onResult: (Boolean) -> Unit) {
        BiometricPrompt
            .Builder(activity)
            .setTitle("Unlock the console")
            .setSubtitle("Proves this phone to the Mac")
            .setAllowedAuthenticators(
                BiometricManager.Authenticators.BIOMETRIC_STRONG or
                    BiometricManager.Authenticators.DEVICE_CREDENTIAL,
            ).build()
            .authenticate(
                CancellationSignal(),
                activity.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(
                        result: BiometricPrompt.AuthenticationResult,
                    ) {
                        onResult(true)
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        // Dismissing the prompt arrives here too, so this is the
                        // ordinary "not now" as much as it is a failure.
                        Log.w(TAG, "not unlocked: $message ($code)")
                        onResult(false)
                    }
                },
            )
    }
}
