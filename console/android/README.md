# console (Android)

The agent console on the phone: read what the Claude Code sessions on the Mac are
doing, send them instructions, and answer the questions they are blocked on —
from wherever you are.

It is the eleventh app on `org.xinutec:shell` and looks like the other ten: one
full-screen **WebView**, no address bar, a home-screen icon. What is different is
underneath it. The other ten are things to read, behind a Nextcloud sign-in and a
VPN. This one can start a process that holds the machine's git credentials, its
kubeconfig and its tokens, so it does not authenticate with a session cookie. It
authenticates with a key generated inside the phone's secure element, which no
server anywhere holds a copy of.

⚠ **An enrolled copy of this app is the fleet's control plane.** The runner's
directory allow-list is a guard rail against mistakes, not a boundary against
someone holding the phone. Revoking is one line in `~/.config/agent-console/clients`
and a restart.

## How it authenticates

Both directions are pinned, and neither consults a trust store.

- **Outbound.** `onReceivedClientCertRequest` presents an EC P-256 key generated
  in StrongBox with `setUserAuthenticationRequired`. It is non-exportable by
  construction: there is no file to steal and nothing a compromised Xinutec server
  can ever hold. The console admits that one public key.
- **Inbound.** `onReceivedSslError` compares the server's public key against
  `BuildConfig.SERVER_PIN` and cancels anything else — which is why the Mac needs
  no publicly-issued certificate, no ACME client and no renewal.

The adversary this is drawn against is the WireGuard hub, which sees every packet
and can forge a source address. It can deny service, which is unavoidable for a
router. It cannot produce either signature, so it cannot read the traffic,
impersonate the phone, or stand in for the Mac.

**One unlock covers five minutes** (`Keys.UNLOCK_SECONDS`), rather than a face scan
per TLS handshake. That number is how long a snatched, unlocked phone stays useful.
A *device* unlock counts, so opening the app after unlocking the phone usually asks
for nothing; when the window has run out the handshake pauses on a biometric prompt
instead of failing.

## Build & install

No toolchain lives in this repo — it borrows the recall project's `android` nix dev
shell (JDK 17 + Android SDK; the Gradle wrapper pins Gradle).

First, on the Mac, give the console a key and write down where it is:

```sh
./scripts/console-identity.sh          # → ~/.config/agent-console/{server.crt,server.key}
```

It prints the two lines that go in `console/android/console.env`, which is
**gitignored** — this repository is public, and the address of a machine on a
private VPN does not belong in it:

```sh
CONSOLE_URL=https://192.168.1.x:8097/
CONSOLE_SERVER_PIN=<64 hex characters>
```

Then build, install and enrol:

```sh
cd console/android
nix develop ~/Code/recall#android --command ./deploy.sh
nix develop ~/Code/recall#android --command ../../scripts/enrol.sh
```

`enrol.sh` generates a challenge, makes the app produce a fresh key for it, pulls
the attestation chain back over adb, and **refuses to pin anything** unless all
seven checks pass — see `console/src/attest.rs`. Then start the console
(`scripts/console.sh`), which picks the material up on its own and binds off
loopback only once there is a pinned client to admit.

Re-enrolling is running `enrol.sh` again: it discards the old key, and the old pin
in `clients` becomes a line to delete.

Runs on Android 11+ (minSdk 30) — higher than the fleet's usual 26, because
`setUserAuthenticationParameters` is what makes the key usable for TLS at all.

## Layout

```
console/android/
├── app/src/main/
│   ├── AndroidManifest.xml                              # INTERNET + USE_BIOMETRIC
│   ├── kotlin/org/xinutec/console/
│   │   ├── MainActivity.kt                              # the two certificate callbacks
│   │   ├── Keys.kt                                      # the StrongBox key and enrolment
│   │   └── Unlock.kt                                    # the prompt, when the window runs out
│   └── res/                                             # launcher icon, theme, strings
├── build.gradle.kts · settings.gradle.kts · gradle/     # project scaffolding
└── deploy.sh                                            # model-guarded install to the Pixel 9
```

A separate Gradle build from `../../android`, not a second module of it: the
console and the viewer are kept apart in every other dimension — separate crate,
separate binary, separate bundle — and one build producing both APKs would be the
one place they met.
