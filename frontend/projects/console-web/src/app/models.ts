/** The console API's wire types. Mirrors `console/src/api.rs` and
 *  `console/src/protocol.rs`; the Rust side is the definition. */

import { Question, Reply } from './questions';

export interface Summary {
  id: string;
  dir: string;
  /** Seconds since the epoch. */
  started: number;
  /**
   * When anything last happened, in **milliseconds**, read from the transcript.
   *
   * ⚠ **Not `started`, which is when the console picked the process up.** For a
   * conversation running since last night the two are thirteen hours apart, and
   * this is the one worth showing: the list is scanned to find which
   * conversation is warm. Absent when the runner cannot find the transcript, so
   * the column is left empty rather than filled with the epoch.
   */
  touched?: number;
  alive: boolean;
  model?: string;
  /** What the CLI last said it was doing, while it is doing anything. */
  busy?: string;
  /**
   * How many times someone has spoken to this session since it was last
   * compacted.
   *
   * ⚠ Exchanges, NOT the `turns` on a turn event — that one counts the
   * assistant messages a single exchange took (5 and 8, for two measured), and
   * summing it answers a question nobody asked. Counted from the transcript by
   * the runner, so it covers the whole conversation rather than however much of
   * it this console happened to watch.
   */
  interactions: number;
  /**
   * What the tokens would have cost at API list prices.
   *
   * ⚠ NOT money — the session runs on the subscription and nothing is billed
   * per token, and at the limit the work waits rather than being charged for.
   * On the details sheet only, labelled as what it is; how much room is left is
   * the utilisation strip's question and it answers it from measurement.
   */
  cost_usd: number;
  /** The account's own rate-limit verdict: allowed, allowed_warning, rejected. */
  limit?: string;
  /** Questions it is blocked on. Nonzero means it cannot go on without you. */
  waiting: number;
  /** The first instruction, kept as the session's name. */
  asked?: string;
  /** What the conversation calls itself — `memview`, `health`. */
  name?: string;
  /** What the session may do without asking. One of `MODES`; absent until the
   *  session has recorded one. */
  mode?: string;
  /** The last request's prompt size in tokens, and the window it went into. */
  context?: number;
  window?: number;
  /**
   * Background tool calls started and not yet reported finished.
   *
   * ⚠ **Only the ones the harness tracks** — a command backgrounded inside a
   * shell announces nothing and is invisible. Absent rather than zero when there
   * are none, so "is anything running" is answered by the field being there.
   */
  background?: number;
  /**
   * How much the transcript weighs, in bytes.
   *
   * ⚠ **Not another way of saying [context].** This is everything said since the
   * conversation began — the turns compaction has since dropped, and every tool
   * result at full length; the context is what the model still has in front of
   * it. The gap is how much has already been forgotten, which is why the details
   * sheet shows both.
   */
  bytes?: number;
}

export interface Overview {
  /** A fingerprint of the bundle the runner is serving. See `Updates`. */
  bundle?: string;
  dirs: string[];
  repos: string[];
  sessions: Summary[];
  /** What the subscription has spent, when a reading has ever arrived. */
  usage?: Usage;
  /**
   * What each conversation is about, by id. Mirrors `gist::Gist`.
   *
   * ⚠ **Written by a model, not read off anything.** Everything else on the list
   * is a fact about a file or a process; this is Haiku's reading of a few
   * thousand characters of transcript. The card marks it as such — a confidently
   * wrong sentence about a conversation nobody has opened is the failure worth
   * avoiding.
   */
  gists?: Record<string, { text: string; at: number }>;
}

/** One rate-limit window. Mirrors `usage::Window`. */
export interface Window {
  /** How much of it is spent, 0–100. */
  pct: number;
  /**
   * How long until it turns over, in milliseconds.
   *
   * ⚠ **Absent means the window has already reset**, which makes `pct` a figure
   * about a window that no longer exists — so it is not a smaller number, it is
   * no number. Decided by the runner against the Mac's clock rather than here:
   * a phone's clock drifts, and this is the one question that must be answered
   * the same way on every screen.
   */
  resets_in_ms?: number;
}

/**
 * The account's rate-limit utilisation. Mirrors `usage::Reading`.
 *
 * Measured by the runner from its own sessions: every `rate_limit_event` the
 * CLI writes carries the figure off the API's response headers. The home
 * dashboard stands behind it for a window nothing has reported yet, and a
 * reading from there can be hours old — which is why `age_ms` is shown rather
 * than kept.
 */
export interface Usage {
  /** Where the reading came from: `this console`, or the dashboard's host. */
  host: string;
  /** How old it is, in milliseconds, by the runner's clock. */
  age_ms: number;
  /**
   * ⚠ **Absent is not the same as expired.** One event names one window, so the
   * runner can know the week and have heard nothing about the five hours. That
   * is no reading — drawn as no row, rather than a row saying something untrue.
   */
  five_hour?: Window;
  seven_day?: Window;
}

/** The event kinds the runner emits, as a value so the wire can be checked
 *  against it rather than trusted. Mirrors `protocol::Event`. */
export const KINDS = [
  'joined',
  'started',
  'prompt',
  'text',
  'tool',
  'tool_result',
  'turn',
  'limit',
  'busy',
  'background',
  'exited',
  'trouble',
  'ask',
  'answered',
  'compacted',
] as const;

export type Kind = (typeof KINDS)[number];

/** One thing that happened in a session. The `kind` discriminates; every other
 *  field depends on it. */
export interface SessionEvent {
  kind: Kind;
  /** When it happened, in milliseconds since the epoch.
   *
   *  A live event is stamped as the runner sees it; a replayed one carries what
   *  the transcript recorded, which for a resumed conversation may be weeks ago.
   *  Absent when a transcript line did not say — never guessed at, because a
   *  guess would date a conversation from June today. */
  readonly at?: number;
  /** `joined` only: how many events above it were read from the transcript. */
  readonly earlier?: number;
  /** `joined` only: the byte offset the seed began at, and the cursor for asking
   *  what came before it. Zero means the seed reached the start of the file. */
  readonly from?: number;
  model?: string;
  cwd?: string;
  tools?: number;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  ok?: boolean;
  /** `tool_result` only: the full length in characters, when `detail` is a cut
   *  of it. Absent means what arrived is the whole of what the tool said. */
  readonly cut?: number;
  cost_usd?: number;
  turns?: number;
  duration_ms?: number;
  stop?: string;
  window?: string;
  status?: string;
  resets_at?: number;
  code?: number;
  detail?: string;
  /** `ask`: the tool it wants to run. `background`: the tool call that started
   *  the task now reported finished. */
  tool?: string;
  title?: string;
  allowed?: boolean;
  /** `answered` only: what was chosen. See `protocol::Event::Answered` for why
   *  it travels with the verdict rather than being kept by whoever chose. */
  reply?: Reply;
}

/** What the transcript is drawn from.
 *
 *  Not the same as an event: consecutive text deltas are one paragraph on
 *  screen, and a tool's result belongs with the call it answers rather than
 *  wherever it happened to arrive. */
export interface Entry {
  /** `day` is not a thing that happened — it is the date the entries after it
   *  fall on, put in by [[fold]] when the conversation crosses midnight. */
  kind: 'said' | 'asked' | 'tool' | 'turn' | 'note' | 'ask' | 'day';
  text: string;
  /** When it happened, in milliseconds since the epoch. For a block built from
   *  several deltas this is when the block *began*, which is what the reader
   *  wants: an answer that took thirty seconds to arrive is filed where it
   *  started, in order with what preceded it. */
  at?: number;
  /** Tool entries only, once the result comes back. */
  ok?: boolean;
  /** Tool entries only: what the tool returned, cut by the runner. */
  detail?: string;
  /** Tool entries only: the first line of [detail], which the row shows without
   *  being asked. Kept apart rather than split at render time — the template
   *  reads it on every change-detection pass. */
  head?: string;
  /** Tool entries only: the full length in characters, when `detail` is a cut. */
  cut?: number;
  tool?: string;
  /** Tool entries only: the call's own id, which is what a result is matched
   *  against. Absent for a transcript line that carried none. */
  call?: string;
  /** `ask` entries only: the control-request id to answer with, and the verdict
   *  once there is one. Undecided is the state that needs a person. */
  ask?: string;
  allowed?: boolean;
  /** `ask` entries only, and only for a question: what it asked, ready to put on
   *  screen as options. Absent for every other tool — and for a question whose
   *  arguments could not be read, which falls back to allow/refuse rather than
   *  showing part of itself. See `questions.ts`. */
  questions?: readonly Question[];
  /** `ask` entries only: what was chosen, once it has been. From the runner
   *  rather than from whichever screen did the choosing, so a second window and
   *  a reloaded one both show it. */
  reply?: Reply;
}

/**
 * A conversation on disk that could be picked up again.
 *
 * The console cannot attach to a running `claude` — a terminal holds its stdin,
 * and one started with `--remote-control` talks to Anthropic with no local
 * endpoint. Resuming its transcript in a process of our own is the nearest thing,
 * and this is what there is to resume.
 */
export interface Conversation {
  /** The session id, which `--resume` takes and which the console keeps. */
  readonly id: string;
  readonly dir: string;
  /** Milliseconds since the epoch. The only proxy for "is this one finished". */
  readonly modified: number;
  readonly bytes: number;
  /** What it calls itself — `music`, `health`. Null when it never took a name. */
  readonly name: string | null;
  /**
   * How full its context was at the last request the transcript records.
   *
   * ⚠ **No window comes with it.** The window's size is declared on the CLI's
   * result line, which never reaches the file — so this is a count and cannot
   * be a fraction. Absent when the tail read found no assistant message.
   */
  readonly context?: number;
  /** Something already has it open, as far as the runner can tell. */
  readonly busy: boolean;
}
