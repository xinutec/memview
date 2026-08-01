package org.xinutec.memview

import org.xinutec.shell.ShellConfig
import org.xinutec.shell.WebShellActivity

/**
 * The memory viewer — the Angular app served at [MEMVIEW_URL], in the fleet's
 * shared [WebShellActivity]. Reachable over the VPN only and behind a Nextcloud
 * sign-in; the WebView keeps the session cookie, so it is a one-time login.
 *
 * There is nothing here but the address. The corpus is read, not operated: no
 * deep link to escape from as the messages archive has, no state to restore
 * beyond the page the shell already remembers, and the graph's pan/pinch are the
 * web app's own.
 */
class MainActivity : WebShellActivity() {
    override val shell =
        ShellConfig(
            url = MEMVIEW_URL,
            // The app plus the Nextcloud login hop. Without the second, the OAuth
            // round-trip is ejected to the browser and the app can never sign in;
            // everything else — a memory linking out — opens in the real browser.
            allowedHosts = setOf("memview.xinutec.org", NC_HOST),
        )

    private companion object {
        // The memory viewer (HTTPS, VPN-only DNS, behind a login).
        const val MEMVIEW_URL = "https://memview.xinutec.org/"

        // The Nextcloud identity provider the login bounces through.
        const val NC_HOST = "dash.xinutec.org"
    }
}
