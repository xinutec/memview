# Agent console — design

A front end for talking to live Claude Code sessions on the Mac: read what they
are doing, send new instructions, approve what they want to run.

**Phase 1 and the approvals half of phase 2 are built** — `console/` (the runner)
and `frontend/projects/console-web` (the UI), run with `scripts/console.sh`. What
is not built is the part the security argument rests on: the client-certificate
gate, and the phone. This document records the decisions and the reasoning
behind them, so the parts that are not obvious from the code do not have to be
re-derived.

## Where it lives

In this repository, as a **workspace member and a second Angular application** —
not its own repository, and not a mode of the viewer. The doc's original
objection was to one *binary* holding two privilege levels, and a separate
`main` answers it: no configuration of the memview pod can turn it into a
console. Three things keep that true, and each is load-bearing:

- `console/` depends on nothing from the `memview` package. Sharing a repository
  is fine; sharing a library would put the privilege boundary inside a crate.
- The image builds `--bin memview` explicitly and copies that one binary, so a
  console binary cannot ride along into a container that runs on isis.
- The UI is its own Angular project with its own bundle, so the console's screens
  are never inside memview's.

The name is settled by the same reasoning: it is *memview's console*, not a
product that needs a brand of its own.

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
Angular 22 zoneless Material, dev-lint gate, self-contained fonts. Copy the spine
rather than reinventing it. It is themed teal against memview's violet on
purpose: the two are open side by side and have different privileges, and the
colour is how you know at a glance which one can run commands.

**Android client** — the Pixel 9. See *Security model*; it is more than the usual
WebView wrapper.

### What phase 1 actually does

`console/src/`: `protocol.rs` reads the CLI's stream-json into a small closed set
of events; `session.rs` owns one subprocess and its transcript; `roster.rs` holds
them all; `api.rs` serves them, streaming with SSE. The UI lists sessions, opens
one, streams its transcript and sends it messages.

Four things were **measured against CLI 2.1.220 rather than assumed**, and each
one changed the design:

- **One process serves many turns.** With `--input-format stream-json` the
  process stays up on an open stdin, keeps one session id across turns, and exits
  0 when stdin closes. So a session is a long-lived subprocess and "send a new
  instruction" is a write, not a cold start.
- **The stream carries a rate-limit event** (`rate_limit_event`, with the window
  and when it resets) — but as a *status*, not a percentage. The statusLine hook
  is still the only source of the percentages, so that part of the design stands.
- **Text arrives twice**: streamed as deltas and repeated in the completed
  message. Taking both shows every answer twice, so text comes from the deltas
  and tool calls from the completed message.
- ⚠⚠ **`--permission-prompt-tool stdio` is the switch that makes approvals
  possible, and `--help` does not mention it.** Found by reading the TypeScript
  SDK, which passes exactly that when given a `canUseTool` callback. Without it a
  `manual` session refuses every tool call outright and records it in
  `permission_denials` — no question ever reaches the client, and an `initialize`
  control request is accepted but changes nothing. With it, the CLI sends
  `control_request`/`can_use_tool` on stdout carrying the tool, its arguments and
  often its own one-line sentence, and waits for a `control_response` of
  `{behavior: allow, updatedInput}` or `{behavior: deny, message}`. The console
  speaks that directly; no SDK and no MCP server in the path.
- ⚠ **In headless mode, the default permission mode refuses every tool call that
  needs permission.** A `Write` in a fresh session came back as an error with no
  file created; the same prompt under `--permission-mode acceptEdits` wrote the
  file. So *phase 1 without the approval channel is a console that can converse
  and little else*. `CONSOLE_PERMISSION_MODE` exposes the choice and defaults to
  nothing: `acceptEdits` is the setting that makes phase 1 useful in a directory
  you trust, `bypassPermissions` hands the machine over, and the console picks
  neither on anybody's behalf. This is the strongest argument for doing phase 2
  next.

### Driving Claude Code

Headless mode is the client protocol:

```
claude -p --input-format stream-json --output-format stream-json \
       --include-partial-messages --replay-user-messages
```

User messages go in as JSON lines on stdin; assistant messages, tool calls, tool
results and partial deltas come back as JSON lines on stdout.

**The runner reads this stream itself, in Rust** — Pippijn's call, 2026-08-02,
over the TypeScript SDK. The SDK is a typed wrapper versioned in lockstep with
the CLI (0.3.220 against 2.1.220), and the cost of declining it was accepted
knowingly: the control protocol behind approvals had to be read off the wire
rather than handed over. It was, and `protocol.rs` speaks it in about thirty
lines. The SDK is still worth *reading* — it is where
`--permission-prompt-tool stdio` was found.

Authentication comes from the subprocess inheriting whatever `claude` already
uses, so sessions stay on the subscription. Building against the Messages API
instead would mean API billing and rebuilding the agent loop.

Relevant flags: `--session-id`, `--resume`, `--fork-session`, `--permission-mode`,
`--permission-prompt-tool stdio` (see above — undocumented and load-bearing),
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
| Compromised device on the home LAN | no | no | yes |
| Stolen Pixel 9, locked | no | no | no |
| Stolen Pixel 9, unlocked | yes | yes | yes |

The last row is the residual risk and it is accepted: the trust anchor is
possession of the phone plus biometric.

**The home LAN is in the table on purpose.** The one-way VPN rules say nothing
about it, and it is not a trusted network: it holds the picades, the heatcam, the
Govee receiver, the phones and the tablet — a dozen devices that receive no
patches and run code nobody audits. A console listening there without
authentication would hand any one of them arbitrary code execution on the
root-of-truth machine, which is a larger hole than the one the VPN rules exist to
close. So the LAN gets the same gate as the VPN and the network is never the
credential. This is why the mTLS work is in phase 1 below rather than phase 2.

**The desk is the one carve-out.** An unauthenticated listener bound to
`127.0.0.1` is sound, because a process already running as this user on the Mac
can spawn `claude` directly and needs no console to do it. Loopback only —
binding `0.0.0.0` for convenience is the mistake this paragraph exists to
prevent.

### Blast radius

The runner's directory allow-list is a guard rail, not a boundary. A session it
does spawn holds the git credentials, the kubeconfig, the Nextcloud session and
the Cloudflare token, so anything that can send instructions to the runner can
reach the whole fleet regardless of which directory the agent was started in.

That is inherent to the feature and is not a reason to abandon it, but it fixes
where the weight sits: the phone's key **is** the fleet's control plane. Hence
the key being hardware-bound and biometric-gated, and revocation being one line
in a config and a restart — the two properties worth paying for.

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

   It only delivers that if the chain is actually checked, and this happens once
   by hand, which is exactly when a step gets skipped. Four things: the
   attestation challenge is freshly generated for this enrolment and appears in
   the record; the chain validates to the Google hardware attestation root; no
   certificate in it is revoked (Google publishes the status list, and the
   revocations that matter are the ones from key extractions); and the record's
   security level says StrongBox with the authentication requirement present.
   Write the check down as a script rather than a paragraph, since it has to run
   again for the iPhone.
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

**Whether a public certificate is needed at all depends on the client shape.**
If the app terminates TLS itself and pins the server's public key — which
*Client shape* below concludes it should, for the StrongBox key — then it is not
validating a name against a public trust store and a self-signed server key is
strictly better: nothing to renew, no Cloudflare token in the path, and no
certificate transparency log recording the name. The DNS-01 certificate is what a
*browser* needs, so it is required only if a WebView or a desktop browser ever
points at a non-loopback address. Decide it with the client, not before.

### Client shape

A WebView can present a client certificate through
`WebViewClient.onReceivedClientCertRequest`, but a StrongBox key that demands user
authentication before every signature makes that handshake awkward to drive. The
app should terminate TLS itself — OkHttp with a custom `KeyManager` — and either
render natively or run a loopback proxy the WebView points at, with the cleartext
exception scoped to `127.0.0.1`.

**Do not take "authentication before every signature" literally.** A key built
with `setUserAuthenticationRequired(true)` and nothing else demands a face unlock
per signature, which is a prompt per TLS handshake and makes the app unusable for
the thing it is for. Use `setUserAuthenticationParameters(N, AUTH_BIOMETRIC_STRONG)`
so one unlock covers a working stretch, and hold a single long-lived connection
for the session's stream rather than a request per exchange — then the handshake,
and the prompt, happen once. `N` is a real security parameter: it is how long a
snatched unlocked phone stays useful, so it belongs in the minutes, not the hours.

This is more than the ~30-line `MainActivity` the other nine apps on
`org.xinutec:shell` use. It is worth the difference here because the security
argument lives in that connection.

## Build order

Each phase is usable on its own and none of the work is thrown away if a later
phase is declined.

1. ✅ **Runner and desk console, on loopback.** Runner on the Mac speaking
   stream-json, Angular UI, bound to `127.0.0.1` — and it *refuses* to bind
   anywhere else, which is a check in `main.rs` rather than a note in a README.
   Sessions may only start inside `CONSOLE_DIRS` (default `~/Code`), resolved
   through symlinks. Port 8097: 8091 is memview's and 8092 was already taken on
   this Mac.
   **Not done in phase 1:** the statusLine socket for the context and rate-limit
   percentages.
2. **Approvals ✅, then the gate.** Approvals are built: a session in
   `--permission-mode manual` asks, the console shows the question, and nothing
   runs until someone answers. What remains of this phase is **mTLS and key
   pinning** — provable with a test key and `curl --cert` before any phone
   exists. Nothing binds off loopback until that passes: the LAN is not a trusted
   network and the runner does not listen on it unauthenticated for a single
   release. With the gate in place the listener moves to the LAN address and the
   phone at home works.
3. **The phone path.** The Android client, StrongBox enrolment and attestation
   check, then the offsite half: the two firewall rules and whatever certificate
   the client shape ends up needing.
4. **memview link-out** from `/agents`.

The old ordering had phase 1 listening on the LAN with no authentication, on the
grounds that pf blocks the VPN and leaves the LAN alone. It does — and the LAN is
the part of the model that had not been stated. See *Security model*.

Do not hardcode the Mac's LAN address in a deployment script. A dead DHCP lease
baked into a deploy script is a failure this fleet has already had.

## Open decisions

- ~~**Name.**~~ Settled: `console`, as a part of memview rather than a product of
  its own, which is why it needs no brand. `steer`, `attend` and `agentctl` were
  brand-checked and rejected — all three are taken, `agentctl` by a control layer
  for coding agents. (`talk` was already ruled out: Nextcloud Talk.)
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
