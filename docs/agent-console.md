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
| Compromised isis | no | no | **yes**, once it is the bridge |
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

**The desk keeps a plaintext loopback socket** (`127.0.0.1:8096`) beside the gated
one, for the same reason loopback-only mode needs no authentication: a process
running as this user can spawn `claude` itself. It is not a convenience — without
it, turning the gate on takes the console away from the machine it runs on, since
the Mac is headless and the SSH forward the desk arrives through has no
certificate to present. A second port because a wildcard bind already covers
loopback on the first.

### Enrolment

No CA and no PKI. Pin raw public keys.

1. Generate the keypair on the device, in StrongBox, biometric-gated.
2. Read the public key and the **Android Key Attestation** chain out once over
   USB. Attestation is what makes this stronger than trust-on-first-use: it is a
   Google-rooted chain asserting the key is hardware-backed and non-exportable on
   a device with a certified keystore, so the enrolment does not rest on the
   phone's own claim about itself.

   It only delivers that if the chain is actually checked, and this happens once
   by hand, which is exactly when a step gets skipped. Written down as
   `console/src/attest.rs` and driven by `scripts/enrol.sh` rather than left as a
   paragraph, since it has to run again for the iPhone. Seven checks, and the
   three added while writing it are as load-bearing as the four that were
   specified:
   - the challenge in the record is the one this enrolment generated on the Mac
     seconds earlier, which is what makes it an answer rather than a recording;
   - every signature in the chain, link by link;
   - the top of the chain is a Google root **held in this repository** — note the
     chain a Pixel 9 produces stops one short of the root, so "it ends at a root"
     is not sufficient as a test;
   - nothing in it is revoked, and a status list that could not be fetched is a
     *failure*, not a skip;
   - the security level is StrongBox for both the record and the key, because a
     StrongBox claim signed at TEE level is a claim by software that does not hold
     the key;
   - the origin is GENERATED, so the key never existed outside the element;
   - and the hardware — not the OS — enforces an authentication requirement.
3. The pinned SPKI hash is a non-secret constant in the runner's config, changed
   by a commit. Revoking is deleting a line and restarting; adding the iPhone
   later is adding a line.

Pin the server's key in the client in the same commit, so neither side will talk
to a substitute.

### Firewall changes required

**Superseded, and reverted.** See *The tunnel*: a Mac that dials out needs no
exception at all. The hub's half was deployed and is gone again (`nixos-config`
`9108db1`); the Mac's half was written and never applied (`xinutec-infra`
`f9f0717`). What follows is kept as the record of what was tried.

**Only for offsite.** On the house Wi-Fi the phone reaches the Mac's LAN address
directly and neither rule is needed — which is why phase 3 splits, and why the
half that costs a firewall exception can be declined on its own.

Both narrow, both in code, both needed only for the phone away from home:

- **amun** — accept forward from `10.100.0.12/32` to the Mac on the console port,
  inserted above the existing drop in `base-configuration.nix`.
- **Mac pf** — a matching `pass in quick` above the
  `block in quick inet from 10.100.0.0/24` rule in `setup-vpn-oneway.sh`.

This weakens the one-way property from *nothing on the VPN may initiate toward
the Mac* to *nothing except the Pixel 9, on one port*. That weakening is why the
mTLS layer carries the weight and these rules do not.

### TLS on the Mac

**Self-signed, and no ACME anywhere.** Settled once the client turned out to pin
the server's key: nothing that connects here consults a trust store, so a
publicly-issued certificate would buy nothing and cost an ACME client on the Mac,
a Cloudflare token in the path, a renewal that can expire at 3am, and the
console's name in a certificate transparency log. `scripts/console-identity.sh`
makes a P-256 key with the Mac's addresses as subjectAltNames and prints the pin
to compile into the app.

The alternative that was on the table — a name pointing at the Mac's VPN address
with a **DNS-01** certificate, the pattern memview already uses for names that
resolve inside the tunnel — is what a *browser* would need. It stays written down
because that is the answer if a desktop browser ever points at a non-loopback
address; DNS-01 rather than HTTP-01 because a host nothing may connect to cannot
be validated inbound.

### Client shape

**It is a WebView wrapper like the other ten, and the reasoning that said it could
not be was wrong.** The original argument ran: a WebView can present a client
certificate through `WebViewClient.onReceivedClientCertRequest`, but a StrongBox
key that demands user authentication before every signature makes that handshake
awkward to drive, so the app should terminate TLS itself with OkHttp and a custom
`KeyManager`, and then either render natively or run a loopback proxy the WebView
points at. Both halves of that turn out not to hold:

- `ClientCertRequest.proceed` takes any `java.security.PrivateKey`, and an
  AndroidKeyStore handle is one. Nothing has to leave the secure element, and no
  second TLS stack is needed. ⚠ The key must permit `DIGEST_NONE` as well as
  SHA-256: Chromium signs through `NONEwithECDSA` with the transcript already
  hashed, and a key allowing only SHA-256 fails inside the network stack where
  nothing says which key or why.
- The awkward half was never about where TLS terminates. **Do not take
  "authentication before every signature" literally** — a key built with
  `setUserAuthenticationRequired(true)` and nothing else demands a face unlock per
  signature, which is a prompt per handshake. `setUserAuthenticationParameters(N,
  AUTH_BIOMETRIC_STRONG or AUTH_DEVICE_CREDENTIAL)` makes one authentication cover
  a stretch. `N` is a real security parameter: it is how long a snatched unlocked
  phone stays useful, so it belongs in the minutes, not the hours. It also means a
  *device* unlock inside the window authorises the key, so opening the app after
  unlocking the phone asks for nothing — which is most of the time.

  ⚠ **Asking whether the key is usable requires actually signing something.**
  `initSign` alone succeeds against a key whose window ran out, because the
  `NONEwithECDSA` implementation buffers its input and opens no keystore operation
  until there is something to sign. The first version of this shipped that mistake:
  measured on a locked Pixel 9, the probe said yes, the certificate was presented,
  the signature failed inside the network stack, and the page failed *without ever
  showing the prompt* — which is the one moment the prompt exists for. Signing 32
  constant bytes and discarding the answer is the test.

The security half of this was confirmed by the same measurement and is worth
stating plainly: a locked phone cannot reach the console. The key refuses, and the
console logs no request.

The server is pinned in the same direction, in `onReceivedSslError`: compare the
SubjectPublicKeyInfo hash of the certificate that arrived against a constant, and
proceed or cancel. That closes the loop symmetrically and is what makes the
question below — whether the Mac needs a publicly-issued certificate — answer
itself.

What this saves is not a few hundred lines of Kotlin. It is a second
implementation of every screen, kept in step with the first for as long as both
exist.

## The tunnel

**Supersedes the firewall exception, 2026-08-02.** Phase 3 reached the phone by
opening one port on the Mac to one VPN peer. That works and was deployed on the
hub, and Pippijn's objection to it was not that it is unsafe but that it need not
exist: *the Mac does not have to be reachable at all*. It can dial out, and then
the one-way rule stands as written rather than as amended.

### The shape

The Mac holds an outbound SSH connection to isis and asks it to listen on its VPN
address. The phone connects there; the bytes go back down the connection the Mac
opened; **the TLS session terminates at the Mac**, at the same pinned-key gate
phase 2 built. isis moves ciphertext and holds no key that opens anything.

    phone ──TLS──▶ isis:8097 ═══tunnel═══▶ Mac 127.0.0.1:8097
                   └─ sees ciphertext ─┘   └─ terminates TLS ─┘

- **`scripts/console-tunnel.sh`** dials it, and `scripts/console.sh` starts it
  alongside the console and kills it with the console. A standing tunnel to a
  console that is not running is a listening port on isis with nothing behind it.
- **isis** sets `GatewayPorts clientspecified` and carries one key restricted to
  `permitlisten` on exactly that address and port — `restrict,port-forwarding`, so
  no shell, no agent, no local forwards. An unattended tunnel on the ordinary
  admin key would give anything holding this Mac's disk a root session there.
- **The Mac binds loopback only**, for both the gated socket and the desk one.

Proven end to end: `openssl s_client` from isis to `10.100.0.2:8097` is answered
by *the Mac's* self-signed certificate, and an unauthenticated `curl` to the same
address gets nothing. The phone reaches it and logs *the console answered with the
pinned key*.

### Why this rather than a message bridge

The first answer to "don't open a port" was a relay on isis holding an encrypted
queue: the phone seals an instruction to the Mac's key, isis stores the sealed
bytes, the Mac dials out and opens them. It is a good design and it is written up
below, because it buys things this does not — store-and-forward, working with no
VPN at all, and a Mac that does nothing but execute.

It was rejected for one reason. **mTLS is audited and a hand-built envelope is
not.** Key agreement, nonce uniqueness, counter persistence across restarts and
two independent implementations agreeing are four places to be quietly wrong, and
being quietly wrong there costs the whole security argument. The passthrough gets
the same "nothing open on the Mac" property while keeping the construction that
has had a decade of adversarial attention. Correctness of the plumbing is a much
easier thing to be confident about than correctness of a protocol.

Worth being honest about what the tunnel does *not* buy: a compromised isis can
still feed bytes to a TLS stack on the Mac, exactly as a compromised amun could
have through the firewall exception. What it removes is the standing exception,
the listening socket, and any exposure at all while the console is stopped.

### The message bridge, if it is ever wanted

Kept because the reasoning is worth not re-deriving, not because it is queued.

isis holds two append-only queues and a bearer token. The phone seals each
instruction to the Mac's key; the Mac long-polls, opens, executes, seals the
result and posts it back. ECIES: static-static ECDH over P-256 between the
phone's StrongBox key and the Mac's, `HKDF-SHA256` twice with different `info`
strings so the two directions never share a nonce space, AES-256-GCM, and a
persisted 64-bit counter per direction forming the nonce and refused when it does
not advance — that last part is what stops the relay replaying an instruction,
which is the one attack a dumb queue is perfectly placed to run.

It carries one requirement that is not obvious and is not negotiable. **Code
delivery is control.** A page served by isis runs on the near side of whatever
signing bridge the app exposes; a compromised isis would ask the StrongBox key to
authorise instructions nobody typed, and the key would agree, because the request
arrives from the app it was enrolled to. End-to-end encryption is no answer when
one end's *code* comes from the adversary, and subresource integrity is no answer
either — whatever would enforce it also arrived from isis. So the bundle would
keep its `memview.xinutec.org` URL and gain a signature the app checks against a
key compiled into the APK, with the signing key on the Mac and a version the app
refuses to move backwards.

That belongs in `org.xinutec:shell` eventually — all eleven wrappers currently run
whatever JavaScript their server returns, which is fine for a viewer behind a
login and would not be fine for `life`, which can spend money — but on the second
consumer, not the first.

## Reaching a conversation that already exists

The console starts processes. **It cannot attach to one**, and that is not a gap
to be closed later — it follows from where the stream lives. A `claude` in a
terminal has its stdin held by the terminal; there is one, and somebody else has
it.

### Remote Control is the other answer, and it is Anthropic's

`claude --remote-control [name]` starts an interactive session that can be driven
from Claude Code on the web or a phone. Measured on a Mac running twelve of them:
outbound HTTPS only, **no listening socket, and no named socket anywhere under
`~/.claude`**. So it is a relay through Anthropic, and Anthropic's own client is
the only thing that can reach those sessions. Nothing local can, this console
included.

That is worth stating plainly rather than treating as a rival: it puts a third
party in the instruction path, which is what this whole design avoids — but not a
*new* one. Anthropic is already trusted with everything a session does, because
the model's output is what drives the tool calls. The console's distinct claim is
narrower than "the only way to reach a session from a phone": it is that the
instruction path is yours end to end, and that approvals are gated by a key in
your phone's secure element.

A session is one or the other. It cannot be both.

### Resume: the same conversation, a process of ours

So reaching an existing conversation means resuming its transcript.
`console/src/past.rs` lists what there is, and `Roster::resume` starts it with
`--resume <id>`, keeping the id so the console's handle and the transcript stay
the same thing.

Two things about reading that list are load-bearing:

- **The transcripts are read, not their filenames decoded.** Claude Code files
  them under `~/.claude/projects/<slug>/<id>.jsonl` where the slug is the working
  directory flattened, and that encoding is undocumented. A guessed one is wrong
  silently and only for the paths nobody tested — a dot in a directory name — and
  the symptom is "there is nothing to resume here" rather than an error. Each
  transcript's own record of its `cwd` is what places it.
- ⚠ **The `cwd` is not on the first line.** It arrives on a `system` line a few
  lines in, which a reader that gives up after one finds never.

The name shown is the conversation's own — `custom-title`, or `agent-name` where
nothing was set by hand — read from the **tail**, because a session is renamed as
its job changes and the name lines are re-emitted every turn. A hex prefix
identifies a transcript; only the name identifies the work.

### Two processes on one transcript

Nothing stops them. Both append, and neither sees the other's turns.

A warning in the UI was the first attempt and it is not a guard — it let a second
process onto a transcript a Remote Control session was still writing. The refusal
now lives in the roster.

**There is no first-party way to ask whether a conversation is in use.** Claude
Code does not hold the transcript open while it runs (`lsof` on a live session's
file returns nothing), writes no lock or pid file, and leaves
`~/.claude/daemon/roster.json` empty for `--remote-control` sessions. All three
checked. So it is inferred from two signals, either of which is enough:

- a running `claude` **names** the session, by id or by the name it currently goes
  by, matched against whole arguments so a directory containing the name cannot
  make it look busy;
- its transcript was **written in the last two minutes**.

Both under-detect, which is the right direction: a false *busy* costs a wait, a
false *free* costs two writers.

⚠ **"a running `claude`" means the executable, not a line mentioning claude.**
Every command Claude Code runs is a shell that first sources a snapshot under
`~/.claude/`, so every one of those command lines carries the substring — and a
process list filtered that way hands over the words of commands that have nothing
to do with any session. A conversation called `utterance` was held as in use by
`grep utterance`. Names are what conversations are chosen by, so the false match
landed precisely where it was most expensive. The first word's last path element
decides now.

The list also has to be **fetched again while somebody is looking at it**. `busy`
is a snapshot of a moment, and the moment it matters is just after a session ends
— fetched once at start-up, the UI still said *in use* long after the process was
gone, which is indistinguishable from the check being wrong. It refreshes on the
same poll as the live sessions, but only while the panel is open.

⚠ **This is why the console must take its sessions down with it.**
`kill_on_drop` needs a destructor to run and a signalled process never runs one,
so stopping the console orphaned every `claude` it had started — and an orphan
keeps its id in the process table, where the check above reads it and refuses to
resume the very conversation nobody is using. `main.rs` handles SIGINT and
SIGTERM.

### What resume does not do yet

**Show you the conversation.** `--resume` restores the CLI's context; it does not
replay past turns on stdout, so the console's view of a resumed session is empty
until something new happens — `0 turns` is the console's count of what it watched,
not the conversation's length. The transcript is on disk and `protocol.rs` already
models the shapes those lines carry, so seeding the view from its tail is the fix.
Not built.

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
2. ✅ **The gate, and approvals.** Both built. A session in
   `--permission-mode manual` asks and nothing runs until someone answers; and
   with `CONSOLE_TLS_CERT`, `CONSOLE_TLS_KEY` and `CONSOLE_CLIENT_KEYS` set the
   console requires a client certificate whose public key is pinned, and only
   then will it bind off loopback. Proven with `curl --cert` against the real
   binary on `0.0.0.0`: the pinned key gets JSON, an unpinned key gets a
   handshake failure, no certificate fails, and plain HTTP to the port gets
   nothing. `console/tests/gate.rs` makes the same four claims by connecting.

   Three things worth knowing before touching it:
   - **`ring`, not the default `aws-lc-rs` provider.** aws-lc wants cmake and a C
     toolchain in every environment that ever builds this; the gate is a handful
     of signatures and does not need a second build system.
   - **The pin is over the key, not the certificate.** A phone's key is generated
     once inside its secure element and cannot be replaced, while its certificate
     may be reissued — pinning the certificate would lock the device out on a
     reissue.
   - **macOS's openssl makes version-1 certificates**, which rustls refuses with
     `UnsupportedCertVersion` and no hint. `Gate::new` checks the version and
     says so, including the `-addext` that fixes it.
3. ✅ **The phone, at home.** `console/android` — the eleventh app on
   `org.xinutec:shell`, with the two certificate callbacks and a StrongBox key.
   `scripts/console-identity.sh` gives the Mac a key of its own,
   `scripts/enrol.sh` makes the phone generate one and refuses to pin it unless
   the attestation chain holds up, and `scripts/console.sh` binds off loopback
   once both exist. Proven on the Pixel 9 over the house Wi-Fi: all seven
   attestation checks passed against a real 5-certificate chain, and the console
   received the web app's own telemetry from the phone — which is a request that
   completed through the gate, so it is the end-to-end claim and not an inference.

   This half costs no firewall change, which is the point of splitting it out.
4. ~~**The phone, away**, by firewall exception.~~ Deployed on the hub and then
   reverted; the Mac's half was never applied. Superseded by *The tunnel*.
5. ✅ **The tunnel.** The Mac dials out to isis, which listens on its VPN address
   and hands the bytes back down; the mTLS gate is untouched and terminates at the
   Mac. Both firewall changes reverted — amun's was deployed and is gone again,
   the Mac's was written and never applied. The Mac now binds loopback only.
   Proven: isis answers with the Mac's certificate, an unauthenticated client gets
   nothing, and the phone reaches it through the tunnel.
6. ✅ **Resume.** Pick up a conversation that already exists rather than only
   starting new ones, refused when anything else appears to be using it. See
   *Reaching a conversation that already exists* — including why the console
   cannot attach to a running session, and why Remote Control can.
   **Not done:** seeding the view from the transcript, so a resumed session shows
   what came before it.
7. **memview link-out** from `/agents`.

The old ordering had phase 1 listening on the LAN with no authentication, on the
grounds that pf blocks the VPN and leaves the LAN alone. It does — and the LAN is
the part of the model that had not been stated. See *Security model*.

Do not hardcode the Mac's LAN address in a deployment script. A dead DHCP lease
baked into a deploy script is a failure this fleet has already had. The phone
build does need *an* address, and it lives in `console/android/console.env` —
outside the repository, beside the pin it belongs with, and written by the script
that generated the key.

## Open decisions

- ~~**Name.**~~ Settled: `console`, as a part of memview rather than a product of
  its own, which is why it needs no brand. `steer`, `attend` and `agentctl` were
  brand-checked and rejected — all three are taken, `agentctl` by a control layer
  for coding agents. (`talk` was already ruled out: Nextcloud Talk.)
- **Approval granularity.** `--permission-mode manual` means a tap per tool call,
  which suits "give them a new instruction" and does not suit watching a long
  build. Options: per-session elevation with a timeout, an allow-list of tool
  patterns, or `acceptEdits` for directories on the allow-list. Still open, and
  now answerable: a tap per call is fine at a desk and the question was always
  what it feels like on a train, which is a thing that can be tried rather than
  reasoned about.
- **Whether the offsite half happens.** The phone works at home for nothing; going
  further buys it away from home, and its price is the pf exception. That is a
  decision to take deliberately, and it is now the only part of it that costs
  anything.
- **What the phone does about a URL that differs by network.** Compiled in today,
  which is fine while the answer is "the house". A name resolving to the LAN
  address at home and the VPN address elsewhere is the obvious fix and is a DNS
  decision, not an app one.

## Deliberately not doing

- **Relaying *decrypted* instructions through a service on isis.** The original
  entry rejected relaying through memview outright, on the grounds that it "puts a
  machine that is exposed to the internet in a position to send arbitrary
  instructions to the root-of-truth host". That is true of a relay that can read
  what it carries and false of one that cannot — so the objection was too broad,
  and *The tunnel* is a relay. What stands is the narrower version: isis may move
  bytes it cannot read, and may not be told what any of them mean.
- **A second WireGuard tunnel nested phone-to-Mac inside the hub tunnel.** It
  reaches the same end-to-end property as mTLS with more moving parts and no
  additional guarantee.
- **Anything built on `~/.claude/daemon/`.** There is a real local IPC there
  (`control.key`, `roster.json`), and it is undocumented and unversioned.
- **The Messages API directly.** It means API billing instead of the subscription,
  and rebuilding the agent loop, tools, MCP, skills and session persistence that
  the CLI already provides.
