# memview (Android)

The memory viewer presented as a native-feeling app: a single full-screen
**WebView**, no address bar, no tabs, a home-screen icon. It avoids browser chrome
while showing the UI exactly as designed (the system WebView is Chromium, so it
renders like Chrome).

The site is **private** — its DNS points inside the WireGuard tunnel — and
**behind a Nextcloud sign-in**. The WebView keeps the session cookie, so it is a
**one-time login**; the app needs only `INTERNET`, since the VPN is set up at the
OS level and not by this app. A phone off the VPN cannot resolve the host at all.

⚠ An installed, signed-in copy of this app is a **standing authenticated session**
over the whole corpus, including the owner-only surfaces (`/agents`, and which
agent wrote each memory). That is the same posture as the messages viewer, but the
corpus is more personal — worth knowing before the phone leaves the house.

## What it does

Almost nothing, which is the point: everything a wrapper does belongs to
`org.xinutec:shell` (see `~/Code/ui-harness/android`), so all that is left here is
the address and the login hop.

- Loads `https://memview.xinutec.org/` — **hardcoded** (`MainActivity.MEMVIEW_URL`);
  this app is single-purpose.
- `allowedHosts` names the app **and `dash.xinutec.org`**. Host confinement is on
  by default, and without the identity provider on that list the OAuth round-trip
  is ejected to the browser and the app can never sign in. Everything else — a
  memory that links out — opens in the real browser.
- The shell handles the rest: insets including the keyboard, bars painted from the
  page's own surface colour so they track light/dark, Back through the SPA history,
  and reopening on the last in-app page.

Runs on any Android 8+ (minSdk 26) device.

## Build & install

No toolchain lives in this repo — it borrows the recall project's `android` nix dev
shell (JDK 17 + Android SDK; the Gradle wrapper pins Gradle). `deploy.sh` does
both, and keys on the device *model* rather than an IP, because DHCP drifts and a
bare `adb install` can hit the wrong connected phone:

```sh
cd android
nix develop ~/Code/recall#android --command ./deploy.sh
```

It tries the VPN address (`10.100.0.12:5555`, the stable one) before the LAN lease.
To build without installing:

```sh
nix develop ~/Code/recall#android --command ./gradlew :app:assembleDebug
# → app/build/outputs/apk/debug/app-debug.apk
```

The APK is signed with the auto-generated debug key — fine for sideloading, the
only distribution path.

## Layout

```
android/
├── app/
│   ├── build.gradle.kts                                 # android app module, no Compose/AppCompat
│   └── src/main/
│       ├── AndroidManifest.xml                          # INTERNET; single launcher activity
│       ├── kotlin/org/xinutec/memview/MainActivity.kt   # url + the login hop, and nothing else
│       └── res/                                         # launcher icon (the web mark), theme, strings
├── build.gradle.kts · settings.gradle.kts · gradle/      # project scaffolding
└── gradlew                                              # borrows ~/Code/recall#android for the SDK
```

The launcher icon is the web app's own `frontend/public/icon.svg` — a link graph on
deep purple — ported to a vector drawable; see the comment in
`res/drawable/ic_launcher_foreground.xml` for why it is a port and not a copy.
