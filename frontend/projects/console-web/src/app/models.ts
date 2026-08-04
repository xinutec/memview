/** The console API's wire types. Mirrors `console/src/api.rs` and
 *  `console/src/protocol.rs`; the Rust side is the definition. */

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
   * per token. Shown only when `limit` says the account has stopped being
   * all-you-can-eat; see `costMatters`.
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
}

export interface Overview {
  /** A fingerprint of the bundle the runner is serving. See `Updates`. */
  bundle?: string;
  dirs: string[];
  repos: string[];
  sessions: Summary[];
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
  /** `ask` entries only: the control-request id to answer with, and the verdict
   *  once there is one. Undecided is the state that needs a person. */
  ask?: string;
  allowed?: boolean;
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
  /** Something already has it open, as far as the runner can tell. */
  readonly busy: boolean;
}
