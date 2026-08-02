# Agent console — design

A front end for talking to live Claude Code sessions on the Mac: read what they
are doing, send new instructions, approve what they want to run.

Nothing here is built. This document records the decisions and the reasoning
behind them, so the parts that are not obvious from the code do not have to be
re-derived. Working name for the new app is `console`; see *Open decisions*.

## Scope, and what stays in memview

memview keeps the watching half and does not gain the talking half.

memview already answers "what are the Claudes doing": `/agents` (which named
session works where), `src/doing.rs` (the timeline, with a verdict per stretch of
work), `/api/work?q=` (who owns a subtree). That is the distilled view and it
should keep growing there.

The talking half goes in a separate Mac-side app, for three reasons.

**Process locality.** memview runs as a pod on isis against a corpus that
`scripts/sync.sh` pushes to it. `claude` has to run on the Mac, in the
checkouts, with the credentials. isis has none of that, so a Mac-side runner
process exists whichever app owns the UI.

**Privilege.** memview is read-only over documents. A console is arbitrary code
execution on the root-of-truth machine. Keeping them in one binary means the two
privilege levels are one environment variable apart, and that is a mistake that
only has to happen once.

**The display rule.** `feedback_memview_distils_never_serves_history` was lifted
in part on 2026-08-02 — timelines are allowed — and the surviving half is *never
verbatim*: no prompt, no reply, no command line, no output. `src/doing.rs`
carries that rule in its module documentation. A console is verbatim by
definition, and `--resume` puts a past session's scrollback one tap from a live
one. That is the path back to the `/history` page that was cut after it shipped.

**What memview does gain:** a link out. `/agents` rows get a *talk to this one*
link to the console's URL for that session. A deep link, not an embedded pane.

When `console` has its own repository, this document moves there and memview
keeps only the link-out.

## Shape

Three pieces.

**Runner** — a service on the Mac that owns Claude Code subprocesses. One
subprocess per session. It speaks the SDK protocol downward and a small JSON API
upward. It is the only component that can start, stop, or send input to a
session, and it is the policy point for what it will accept.

**Console UI** — Angular, served by the runner. Same stack as memview: axum +
Angular 22 zoneless Material, `@xinutec/ui-harness`, dev-lint gate, self-contained
fonts. Copy the spine rather than reinventing it.

**Android client** — the Pixel 9. See *Security model*; it is more than the usual
WebView wrapper.

### Driving Claude Code

Headless mode is the client protocol:

```
claude -p --input-format stream-json --output-format stream-json \
       --include-partial-messages --replay-user-messages
```

User messages go in as JSON lines on stdin; assistant messages, tool calls, tool
results and partial deltas come back as JSON lines on stdout. Use
`@anthropic-ai/claude-agent-sdk` rather than parsing the stream by hand — it is a
typed wrapper over this protocol and is versioned in lockstep with the CLI
(checked 2026-08-02: SDK 0.3.220 against CLI 2.1.220).

Authentication comes from the subprocess inheriting whatever `claude` already
uses, so sessions stay on the subscription. Building against the Messages API
instead would mean API billing and rebuilding the agent loop.

Relevant flags: `--session-id`, `--resume`, `--fork-session`, `--permission-mode`,
`--settings`, `--include-hook-events`, `--forward-subagent-text`.

### Where the percentages come from

Two different numbers, and they do not arrive the same way.

`claude -p --output-format json` returns `usage`, `modelUsage`, `total_cost_usd`,
`num_turns`, `stop_reason` — enough to derive context usage — and **no
`rate_limits`** (probed 2026-08-02 against CLI 2.1.220; this matches the finding
recorded when the home dashboard was built).

The statusLine hook gets both, precomputed. The `xinutec-infra` repo's
`mac-mini/claude-usage-statusline.py`
reads `context_window.used_percentage` (line 64) and
`rate_limits.{five_hour,seven_day}` (line 67) from the JSON Claude Code pipes to
it on stdin.

So the runner spawns each session with `--settings` carrying its own `statusLine`
command: a small writer that forwards that JSON to the runner over a unix socket.
This is a documented hook, it fires on every status refresh, and it hands over
context %, 5-hour % and weekly % without the runner computing anything.

The existing `settings.json` statusLine must keep working for interactive
sessions on the Mac; the runner's `--settings` applies to its own subprocesses
only.

## Security model

The Mac is a one-way WireGuard peer and that is deliberate: it is the
root-of-truth for irreplaceable archives, and the property being protected is
integrity, not confidentiality. A compromised internet-facing server must not be
able to destroy it. Adding a console is adding a way to execute instructions on
that machine, so the model has to be stated rather than assumed.

### What exists now

- **Mac pf** (`xinutec-infra`, `mac-mini/setup-vpn-oneway.sh`):
  `pass out quick inet to 10.100.0.0/24 keep state` +
  `block in quick inet from 10.100.0.0/24`.
- **Servers** (`nixos-config`, `base-configuration.nix:181-186`): OUTPUT drops
  `ctstate NEW` toward the Mac's VPN address; FORWARD accepts
  `ESTABLISHED,RELATED` and drops the rest.
- **WireGuard is hub-and-spoke**: the hub holds `allowedIPs = [ "${node.vpn}/32" ]`
  per peer (`base-configuration.nix:248`), peers hold the whole subnet toward the
  hub (`:260`). Pixel 9 is `10.100.0.12` (`network.nix:80-85`), the Mac is
  `10.100.0.11` (`network.nix:102-106`).

Two consequences that shape everything below. Because the hub decrypts and
re-encrypts, **amun sees the plaintext of all peer-to-peer traffic today**. And
because only `ctstate NEW` is dropped toward the Mac, an outbound-initiated
connection is permitted by design rather than by loophole.

### The adversary the design targets

| Adversary | Read traffic | Inject or impersonate | Deny service |
| --- | --- | --- | --- |
| Compromised isis | no | no | no |
| Compromised amun (hub) | no | no | yes |
| Stolen Pixel 9, locked | no | no | no |
| Stolen Pixel 9, unlocked | yes | yes | yes |

The last row is the residual risk and it is accepted: the trust anchor is
possession of the phone plus biometric.

### How that is achieved

**The firewall is not the authentication.** A source-address allow is defeated by
a compromised hub, which can forge sources. It stays as a first filter — it means
only one address can open a socket at all, so an attacker holding amun cannot
reach the TLS stack to probe it — but it authenticates nothing.

**Mutual TLS with a hardware-bound client key is the gate.** An EC keypair is
generated in the Pixel 9's StrongBox with `setUserAuthenticationRequired(true)`.
It is non-exportable by construction, so there is no file to steal and nothing
amun can ever hold. The runner requires a client certificate and accepts one
public key.

Against a compromised amun this leaves: no read (TLS ciphertext inside WireGuard
ciphertext), no injection, no impersonation — it cannot sign the
`CertificateVerify` — and full denial of service, which is unavoidable for the
router and is the acceptable half. It also learns metadata: that the phone talked
to the Mac, roughly when, roughly how much.

**Permission mode.** The runner spawns sessions with `--permission-mode manual`
and surfaces each request for approval. With the phone as the trust anchor the
approval travels the same channel as the instruction, which is sound because that
channel is authenticated end to end by a key amun cannot produce. The cost is
that every tool call needs a tap; see *Open decisions*.

**Session and directory allow-list.** The runner refuses to spawn outside a list
of directories held in its config. A client that is somehow trusted still cannot
start an agent anywhere on the disk.

### Enrolment

No CA and no PKI. Pin raw public keys.

1. Generate the keypair on the device, in StrongBox, biometric-gated.
2. Read the public key and the **Android Key Attestation** chain out once over
   USB. Attestation is what makes this stronger than trust-on-first-use: it is a
   Google-rooted chain asserting the key is hardware-backed and non-exportable on
   a device with a certified keystore, so the enrolment does not rest on the
   phone's own claim about itself.
3. The pinned SPKI hash is a non-secret constant in the runner's config, changed
   by a commit. Revoking is deleting a line and restarting; adding the iPhone
   later is adding a line.

Pin the server's key in the client in the same commit, so neither side will talk
to a substitute.

### Firewall changes required

Both narrow, both in code, both needed only for the phone path (phase 3):

- **amun** — accept forward from `10.100.0.12/32` to the Mac on the console port,
  inserted above the existing drop in `base-configuration.nix`.
- **Mac pf** — a matching `pass in quick` above the
  `block in quick inet from 10.100.0.0/24` rule in `setup-vpn-oneway.sh`.

This weakens the one-way property from *nothing on the VPN may initiate toward
the Mac* to *nothing except the Pixel 9, on one port*. That weakening is why the
mTLS layer carries the weight and these rules do not.

### TLS on the Mac

The Mac has no certificate and Android refuses cleartext. Use the issuer pattern
memview already uses for names that resolve inside the tunnel: an `A` record for
the console's name pointing at the Mac's VPN address, with a **DNS-01**
certificate. DNS-01 needs no inbound reachability, which is why it fits a host
nothing may connect to (it is also why memview's `Exposure = VpnOnly` forces it —
HTTP-01 cannot validate such a name and fails as a certificate pending forever).

The Mac is not in k8s, so this is acme on the Mac with the Cloudflare token, not
cert-manager.

### Client shape

A WebView can present a client certificate through
`WebViewClient.onReceivedClientCertRequest`, but a StrongBox key that demands user
authentication before every signature makes that handshake awkward to drive. The
app should terminate TLS itself — OkHttp with a custom `KeyManager` — and either
render natively or run a loopback proxy the WebView points at, with the cleartext
exception scoped to `127.0.0.1`.

This is more than the ~30-line `MainActivity` the other nine apps on
`org.xinutec:shell` use. It is worth the difference here because the security
argument lives in that connection.

## Build order

Each phase is usable on its own and none of the work is thrown away if a later
phase is declined.

1. **Runner and LAN-direct console.** Runner on the Mac speaking stream-json,
   statusLine socket wired for the percentages, Angular UI, reachable on the Mac's
   LAN address. No firewall change, no certificate, no threat-model decision. This
   already covers the desk and the phone while at home, since pf blocks the VPN
   range and leaves the LAN alone — the same reason `scripts/dev.sh` is reachable
   at the Mac's LAN address today.
2. **mTLS, key pinning, permission-mode approvals.** Still LAN-only. Everything
   the offsite path needs, provable without touching the firewall.
3. **The phone path.** DNS-01 certificate, the two firewall rules, the Android
   client, StrongBox enrolment.
4. **memview link-out** from `/agents`.

Do not hardcode the Mac's LAN address in a deployment script. A dead DHCP lease
baked into a deploy script is a failure this fleet has already had.

## Open decisions

- **Name.** `console` is a working name only. The convention is a plain
  descriptive word, never a deity, and it must be brand-checked against existing
  products and against names already used in the fleet before it is adopted.
  `talk` is ruled out — it collides with Nextcloud Talk.
- **Approval granularity.** `--permission-mode manual` means a tap per tool call,
  which suits "give them a new instruction" and does not suit watching a long
  build. Options: per-session elevation with a timeout, an allow-list of tool
  patterns, or `acceptEdits` for directories on the allow-list. Pick after using
  phase 1, not before.
- **Whether phase 3 happens at all.** Phases 1 and 2 give the desk and the house.
  Phase 3 buys offsite, and its price is the pf exception. That is a decision to
  take deliberately.

## Deliberately not doing

- **Relaying through memview on isis.** An outbound connection the Mac dials to
  isis works within the current firewall rules, and it was rejected anyway: it
  puts a machine that is exposed to the internet in a position to send arbitrary
  instructions to the root-of-truth host. The Android client removes isis from
  the path entirely, which is a stronger answer than trying to make a compromised
  isis safe.
- **A second WireGuard tunnel nested phone-to-Mac inside the hub tunnel.** It
  reaches the same end-to-end property as mTLS with more moving parts and no
  additional guarantee.
- **Anything built on `~/.claude/daemon/`.** There is a real local IPC there
  (`control.key`, `roster.json`), and it is undocumented and unversioned.
- **The Messages API directly.** It means API billing instead of the subscription,
  and rebuilding the agent loop, tools, MCP, skills and session persistence that
  the CLI already provides.
