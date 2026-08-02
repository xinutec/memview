package org.xinutec.console

import android.content.Intent
import android.graphics.Color
import android.net.http.SslError
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.webkit.ClientCertRequest
import android.webkit.SslErrorHandler
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.widget.FrameLayout
import android.widget.TextView
import org.xinutec.shell.ShellConfig
import org.xinutec.shell.WebDebugging
import org.xinutec.shell.WebShellActivity
import java.security.cert.X509Certificate

/**
 * The agent console on the phone: the same Angular UI the desk uses, behind a
 * handshake only this device can complete.
 *
 * It is a WebView wrapper like the other ten, and that was not the plan. The design
 * assumed a WebView could not carry a hardware-bound, user-authenticated key and
 * concluded the app should terminate TLS itself and either render natively or run a
 * loopback proxy. It can: [ClientCertRequest.proceed] takes any
 * `java.security.PrivateKey`, an AndroidKeyStore handle included, and the awkward
 * half — a face scan per signature — is answered by the key's own authentication
 * window rather than by moving the TLS stack. What that saves is not a few hundred
 * lines of Kotlin but a second implementation of every screen.
 *
 * ## Both directions are pinned, so nothing here trusts a name
 *
 * - **Outbound**, [ConsoleWebViewClient.onReceivedClientCertRequest] presents the
 *   secure-element key from [Keys]. The console admits that public key and no
 *   other.
 * - **Inbound**, [ConsoleWebViewClient.onReceivedSslError] compares the server's
 *   public key against [BuildConfig.SERVER_PIN] and refuses anything else — which
 *   is why the Mac needs no publicly-issued certificate. Nothing here consults a
 *   trust store, so there is no certificate to renew, no ACME client on the Mac, no
 *   Cloudflare token in the path and no name in a transparency log.
 *
 * The hostile party this is drawn against is the WireGuard hub, which sees every
 * packet in clear and can forge a source address. It cannot produce either
 * signature, so it cannot read the traffic, impersonate the phone, or stand in for
 * the Mac. It can still deny service, which is unavoidable for a router.
 *
 * ## What it does NOT protect against
 *
 * A stolen, unlocked phone within the key's authentication window. That is what
 * [Keys.UNLOCK_SECONDS] is sized against, and why it is minutes.
 */
class MainActivity : WebShellActivity() {
    override val shell =
        ShellConfig(
            url = BuildConfig.CONSOLE_URL.ifEmpty { UNCONFIGURED_URL },
            consoleTag = "console-web",
            webDebugging = WebDebugging.DEBUG_BUILDS,
        )

    /** The tap-to-retry notice, present only while something is wrong. */
    private var notice: TextView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (BuildConfig.CONSOLE_URL.isEmpty() || BuildConfig.SERVER_PIN.isEmpty()) {
            show(
                "This build was not told where the console is.\n\n" +
                    "Set CONSOLE_URL and CONSOLE_SERVER_PIN and build again — " +
                    "see console/android/README.md.",
            )
        }
        enrol(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        enrol(intent)
    }

    // Nothing deep-links into the console, and an intent is an external input: a
    // wrapper with no address bar gives no way to notice it is showing somewhere
    // else. The enrolment intent carries no URL — see `enrol`.
    override fun startUrl(intent: Intent?): String? = null

    override fun createWebViewClient() = ConsoleWebViewClient()

    inner class ConsoleWebViewClient : ShellWebViewClient() {
        /**
         * Present this phone's key, unlocking it first if its window has expired.
         *
         * The request is answered asynchronously when a prompt is needed, which is
         * what keeps the face scan out of the ordinary path: a device unlock inside
         * the last [Keys.UNLOCK_SECONDS] already authorises the key, so opening the
         * app straight after unlocking the phone asks for nothing.
         */
        override fun onReceivedClientCertRequest(view: WebView, request: ClientCertRequest) {
            // Logged on the way in, not only on the way out. Both halves of this
            // handshake are silent when they work and produce a bare "handshake
            // failed" in the Chromium log when they do not, so which callback ran
            // at all is the first question worth being able to answer.
            Log.i(TAG, "client certificate wanted by ${request.host}:${request.port}")
            val key = Keys.privateKey()
            val leaf = Keys.leaf()
            if (key == null || leaf == null) {
                Log.e(TAG, "asked for a client certificate and this phone has no key — enrol it")
                show("This phone is not enrolled yet.\n\nRun scripts/enrol.sh on the Mac.")
                request.cancel()
                return
            }
            // The leaf alone. The rest of the chain is the attestation record, which
            // is enrolment's business and has nothing to say to a TLS server that
            // pins one public key.
            val chain = arrayOf<X509Certificate>(leaf)
            if (Keys.unlocked()) {
                request.proceed(key, chain)
                return
            }
            Unlock.prompt(this@MainActivity) { ok ->
                if (ok) {
                    request.proceed(key, chain)
                } else {
                    request.cancel()
                    show("Locked.\n\nTap to unlock and try again.")
                }
            }
        }

        /**
         * Accept the Mac's certificate if and only if it carries the pinned key.
         *
         * The error being handled is "untrusted issuer", which is what a self-signed
         * certificate always produces and is not the question worth asking. The
         * question worth asking — is this the key we agreed on — no trust store can
         * answer, so it is answered here.
         */
        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
            Log.i(TAG, "server certificate query: error ${error.primaryError} on ${error.url}")
            val certificate = error.certificate.x509Certificate
            if (certificate == null) {
                Log.e(TAG, "TLS error with no certificate to inspect: ${error.primaryError}")
                handler.cancel()
                return
            }
            val pin = Keys.pin(certificate)
            if (pin == BuildConfig.SERVER_PIN) {
                Log.i(TAG, "the console answered with the pinned key")
                handler.proceed()
                return
            }
            // Logged with the fingerprint, because the only two things this can mean
            // are "the Mac's key was replaced and this build is stale" and "somebody
            // is standing in the middle", and the fingerprint is what tells them
            // apart.
            Log.e(TAG, "refused the server: key $pin is not the pinned one")
            handler.cancel()
            show("The console answered with a key this app does not know.\n\n$pin")
        }

        override fun onReceivedError(
            view: WebView,
            request: WebResourceRequest,
            error: WebResourceError,
        ) {
            super.onReceivedError(view, request, error)
            // Subresource failures are the page's business; a main-frame failure means
            // there is nothing on screen to explain itself.
            if (!request.isForMainFrame) return
            show("Not connected.\n\n${error.description}\n\nTap to try again.")
        }

        override fun onPageFinished(view: WebView, url: String) {
            super.onPageFinished(view, url)
            if (url != UNCONFIGURED_URL) hide()
        }
    }

    /**
     * Generate a fresh key for a challenge that came from the Mac.
     *
     *     adb shell am start -n org.xinutec.console/.MainActivity --es enrol_challenge <hex>
     *
     * Driven from outside on purpose: a challenge the phone chose for itself would
     * make the attestation record a claim it could have prepared in advance, and the
     * record is the only reason to believe the key is where it says it is.
     */
    private fun enrol(intent: Intent?) {
        val hex = intent?.getStringExtra(EXTRA_CHALLENGE) ?: return
        // Once. onCreate and onNewIntent can both see the same intent, and enrolling
        // twice would throw away the key the first one just published.
        intent.removeExtra(EXTRA_CHALLENGE)
        try {
            val challenge = hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            require(challenge.size >= MIN_CHALLENGE) { "challenge is too short to be fresh" }
            Keys.enrol(challenge)
            val file = Keys.writeEnrolment(filesDir)
            val pin = Keys.leaf()?.let(Keys::pin)
            Log.i(TAG, "enrolled $pin — chain written to $file")
            show("Enrolled.\n\n$pin\n\nFinish on the Mac.")
            // The old key's decision for this host is cached inside the network stack,
            // so without this the next handshake presents the certificate that was
            // just discarded.
            WebView.clearClientCertPreferences { web.reload() }
        } catch (failure: Exception) {
            Log.e(TAG, "enrolment failed", failure)
            show("Enrolment failed.\n\n$failure")
        }
    }

    /** Cover the page with something that says what is wrong; tapping retries. */
    private fun show(text: String) {
        notice?.let { root.removeView(it) }
        val panel =
            TextView(this).apply {
                setText(text)
                setPadding(PAD, PAD, PAD, PAD)
                setBackgroundColor(BACKDROP)
                setTextColor(Color.WHITE)
                textSize = 16f
                gravity = Gravity.CENTER
                layoutParams =
                    FrameLayout.LayoutParams(
                        FrameLayout.LayoutParams.MATCH_PARENT,
                        FrameLayout.LayoutParams.MATCH_PARENT,
                    )
                setOnClickListener {
                    hide()
                    web.reload()
                }
            }
        notice = panel
        root.addView(panel)
    }

    private fun hide() {
        notice?.let { root.removeView(it) }
        notice = null
    }

    private companion object {
        const val TAG = "console-app"
        const val EXTRA_CHALLENGE = "enrol_challenge"

        // 16 bytes. Short enough to type, long enough that a phone cannot have
        // guessed it in advance, which is the only property the challenge needs.
        const val MIN_CHALLENGE = 16

        // Unresolvable by construction (RFC 2606 reserves .invalid), so a build that
        // was never told its address fails in the notice rather than by connecting to
        // whatever answers.
        const val UNCONFIGURED_URL = "https://console.invalid/"

        const val PAD = 48
        const val BACKDROP = 0xFF12121AL.toInt()
    }
}
