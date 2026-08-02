package org.xinutec.console

import android.content.Intent
import android.graphics.Color
import android.net.http.SslError
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.webkit.ClientCertRequest
import android.webkit.JavascriptInterface
import android.webkit.SslErrorHandler
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.widget.FrameLayout
import android.widget.TextView
import androidx.lifecycle.Lifecycle
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

    /**
     * Whether the page on screen is a failure rather than the console.
     *
     * Drives the retry in [onResume], which exists because the commonest failure
     * here heals itself while the app is in the background: the key's
     * authentication window runs out, the handshake is refused, and then the phone
     * is unlocked to look at it — at which point the key works again and the app
     * is still showing the error it hit a minute ago. Waiting to be told to retry,
     * by someone who can see nothing but the error, is the wrong way round.
     */
    private var failed = false

    /** Whether a prompt is already on screen — see [renewKey]. */
    private var prompting = false

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

    override fun onResume() {
        super.onResume()
        // Only after a failure: a reload on every resume would throw away the
        // session's scroll position and re-open the event stream every time the
        // app is glanced at.
        if (failed) {
            retry()
            return
        }
        // ⚠ A page that loaded fine still stops working when the key's window
        // runs out. Chromium caches the client-certificate decision per host, so
        // later handshakes reuse the key handle WITHOUT asking the app again —
        // `onReceivedClientCertRequest` never runs, nothing prompts, and every
        // request the page makes fails in silence. Measured on the Pixel 9:
        //
        //     cr_AndroidKeyStore: UserNotAuthenticatedException: User not authenticated
        //     ssl_client_socket_impl.cc: handshake failed; net_error -141
        //
        // with the page above it reporting that the Mac was not answering, while
        // the Mac was answering perfectly. So the window is renewed here, where
        // there is a person present to renew it: coming back to the app is the
        // moment before the key is needed again.
        if (!Keys.unlocked()) renewKey()
    }

    /**
     * Put the key back inside its authentication window, silently if refused.
     *
     * No `show` on failure: this runs when the app comes forward, not in answer
     * to anything, and a phone declining to scan a face is not an error worth
     * covering the page with — the page is still there, and the next request will
     * ask again.
     */
    private fun renewKey() {
        // One prompt at a time. The page polls every few seconds and every failed
        // poll asks again, so without this the first lapse stacks prompts faster
        // than anybody can answer them.
        if (prompting) return
        // A prompt needs a window to appear in. The page can ask from the
        // background — a poll that fires as the app is going away — and a
        // BiometricPrompt raised then is at best invisible.
        if (!lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return
        prompting = true
        Unlock.prompt(this) { ok ->
            prompting = false
            Log.i(TAG, if (ok) "key renewed" else "key left locked")
        }
    }

    /**
     * Let the page ask for the key to be renewed.
     *
     * ⚠ The page cannot diagnose its own silence. A refused client certificate
     * and an unreachable Mac both arrive as a request that was never answered,
     * and only this side knows which it was — so the page asks, and this decides.
     *
     * The surface is one method that takes nothing and returns nothing, because a
     * bridge into a WebView is reachable by whatever the WebView has loaded. The
     * shell confines that to the console's own host, the console answers only on a
     * pinned certificate, and even so the most this can be made to do is show a
     * prompt the person then refuses.
     */
    override fun onWebViewCreated(web: WebView) {
        super.onWebViewCreated(web)
        web.addJavascriptInterface(Bridge(), "consoleHost")
    }

    inner class Bridge {
        @JavascriptInterface
        fun renew() {
            // On the UI thread: this arrives on the WebView's JavaScript thread,
            // and a prompt raised from there never appears.
            runOnUiThread {
                // Checked, not assumed. Most nothing-answered is an asleep Mac or a
                // dropped tunnel, and prompting for those would train the answer
                // "dismiss" into a control whose whole purpose is to be read.
                if (!Keys.unlocked()) renewKey()
            }
        }
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
                    show("Locked.\n\nUnlock the phone, or tap here.")
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
            // A load that reached the console clears both the notice and the reason
            // to retry. Chromium's error page finishes too, so this cannot be the
            // only place `failed` is written — the callbacks that set it run first.
            if (!failed && url != UNCONFIGURED_URL) hide()
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

    /** Load the console again, from whatever went wrong. */
    private fun retry() {
        failed = false
        hide()
        web.reload()
    }

    /** Cover the page with something that says what is wrong; tapping retries. */
    private fun show(text: String) {
        failed = true
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
                setOnClickListener { retry() }
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
