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
}

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
