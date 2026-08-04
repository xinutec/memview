plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

// Where this build's console lives, and which server key it will speak to. Both
// are deployment facts, not source: this repository is public, and the address of
// a machine on a private VPN does not belong in it. They arrive from the
// environment — `deploy.sh` sources `console.env` (gitignored) — and default to
// empty, in which case the app says which two variables to set instead of dialling
// somewhere wrong.
val consoleUrl: String = System.getenv("CONSOLE_URL") ?: ""
val serverPin: String = System.getenv("CONSOLE_SERVER_PIN") ?: ""

/**
 * The console's own certificate, carried into the app as a trust anchor.
 *
 * ⚠ **Without it every connection begins as an error.** The certificate is
 * privately issued and in no store the phone has, so Chromium fails validation
 * on each one — and the only thing that rescues a request is
 * `onReceivedSslError`, which WebView calls for some kinds and not others.
 * Measured on the Pixel: 233 handshakes failed `ERR_CERT_AUTHORITY_INVALID`
 * without the app being consulted once, one per five-second poll, while the page
 * itself worked perfectly through the callback. A wasted handshake every five
 * seconds is battery and latency — and it is also the noise a real failure would
 * hide in.
 *
 * **This does not replace the pin.** `MainActivity` still checks the key it is
 * given against [serverPin], which is a stronger statement than "some anchor
 * signed it". This only stops the network stack from objecting first.
 *
 * Registered as a task *input*, so changing the certificate rebuilds the APK
 * rather than shipping one that trusts a key the console no longer has.
 */
val consoleHome: String =
    System.getenv("CONSOLE_HOME") ?: "${System.getProperty("user.home")}/.config/agent-console"
val serverCert = File(consoleHome, "server.crt")

/** The host to scope that anchor to — never wider than the one console. */
val consoleHost: String =
    consoleUrl.substringAfter("://").substringBefore("/").substringBefore(":")

val trustDir = layout.buildDirectory.dir("generated/trust")

val writeTrustAnchor by tasks.registering {
    description = "Carry the console's certificate into the app as a scoped trust anchor."
    inputs.file(serverCert).optional(true)
    inputs.property("host", consoleHost)
    outputs.dir(trustDir)
    doLast {
        val root = trustDir.get().asFile
        val xml = File(root, "xml").apply { mkdirs() }
        val raw = File(root, "raw").apply { mkdirs() }
        // Nothing configured: the app already says which variables to set, and a
        // build that cannot dial anywhere has nothing to trust. System anchors
        // only, which is Android's own default.
        if (consoleHost.isEmpty()) {
            File(xml, "network_security_config.xml").writeText(SYSTEM_ONLY)
            File(raw, "console_ca.crt").delete()
            return@doLast
        }
        check(serverCert.isFile) {
            "$serverCert is missing, so the app would trust nothing this console " +
                "presents. Run scripts/console-identity.sh on the Mac first."
        }
        serverCert.copyTo(File(raw, "console_ca.crt"), overwrite = true)
        File(xml, "network_security_config.xml").writeText(anchoredTo(consoleHost))
    }
}

/** Android's own default, for a build with no console to speak to. */
val SYSTEM_ONLY =
    """
    <?xml version="1.0" encoding="utf-8"?>
    <network-security-config>
        <base-config><trust-anchors><certificates src="system" /></trust-anchors></base-config>
    </network-security-config>
    """.trimIndent()

/**
 * Trust the console's certificate **for the console's host and nowhere else.**
 *
 * A `base-config` would have been one line shorter and would have meant the key
 * on that Mac could stand in for any site the app ever loads. It loads one.
 */
fun anchoredTo(host: String) =
    """
    <?xml version="1.0" encoding="utf-8"?>
    <network-security-config>
        <domain-config>
            <domain includeSubdomains="false">$host</domain>
            <trust-anchors>
                <certificates src="@raw/console_ca" />
                <certificates src="system" />
            </trust-anchors>
        </domain-config>
    </network-security-config>
    """.trimIndent()

android {
    namespace = "org.xinutec.console"
    compileSdk = 36
    // Pin to the build-tools the nix SDK provides (AGP would otherwise pick a
    // version that isn't in the read-only SDK).
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "org.xinutec.console"
        // minSdk 30, higher than the fleet's usual 26, and the reason is the whole
        // point of this app: `setUserAuthenticationParameters` — one unlock covering
        // a stretch of time rather than a face scan per signature — arrived in
        // Android 11. Below it the only expressible key is unusable for TLS.
        minSdk = 30
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
        buildConfigField("String", "CONSOLE_URL", "\"$consoleUrl\"")
        buildConfigField("String", "SERVER_PIN", "\"$serverPin\"")
    }

    buildFeatures {
        buildConfig = true
    }

    buildTypes {
        // Sideloaded build — no shrinking, signed with the debug key for simplicity.
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The generated anchor sits beside the hand-written resources rather than
    // being copied into them: what a build produced belongs under build/, and a
    // certificate committed by accident is a thing that cannot be uncommitted.
    sourceSets["main"].res.srcDir(trustDir)
}

tasks.named("preBuild") { dependsOn(writeTrustAnchor) }

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// Say so in a sentence rather than a stacktrace when the shell isn't beside us.
// Resolved against rootDir (console/android/), so this is the same path
// settings.gradle.kts includes — file() here would resolve against app/ and never
// match.
require(rootDir.resolve("../../../ui-harness/android").isDirectory) {
    "ui-harness must be checked out beside this repo (~/Code/ui-harness)"
}

dependencies {
    // The shared WebView shell (ui-harness/android), substituted to a project by
    // settings.gradle.kts. No version, ever: it resolves by path. It brings
    // androidx.activity with it (ComponentActivity is its superclass).
    implementation("org.xinutec:shell")
    // core-ktx for the prefs/insets KTX. No Compose, no AppCompat — and no
    // androidx.biometric either: the framework's own BiometricPrompt needs no
    // FragmentActivity, which the shell is not.
    implementation(libs.androidx.core.ktx)
}
