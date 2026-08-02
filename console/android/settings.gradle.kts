pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

// A separate Gradle build from ../../android, not a second module of it. The
// console and the viewer are kept apart everywhere else — separate crate, separate
// binary, separate bundle — and one build producing both APKs would be the one
// place they met.
rootProject.name = "console-app"
include(":app")

// The shared WebView shell, resolved by path against the checkout beside this
// repo — no publishing, no version, no pin to bump (see ui-harness/android/README.md).
includeBuild("../../../ui-harness/android") {
    dependencySubstitution {
        substitute(module("org.xinutec:shell")).using(project(":main"))
    }
}
