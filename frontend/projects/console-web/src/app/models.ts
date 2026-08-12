/** The console API's wire types. Mirrors `console/src/api.rs` and
 *  `console/src/protocol.rs`; the Rust side is the definition. */

import { Question, Reply } from './questions';

/**
 * One background call, named. Mirrors `protocol::Called`.
 *
 * `label` is the call's own `description` where it has one, else the command or
 * prompt, flattened and cut to 60 characters by the runner — a Bash one-liner
 * here ran to several hundred. Absent when the input carries nothing readable:
 * an unlabelled tool name beats an invented one.
 */
export interface Called {
  tool: string;
  label?: string;
  task?: string;
}

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
  /** Whether a turn is running, as the runner observes it.
   *
   *  ⚠ **Not the absence of [[busy]], which is what this replaces.** A status is
   *  announced only when it changes, so a session running tools for ten minutes
   *  can have nothing standing — and the screens drew that as *idle*. See
   *  `session::Summary::working`. */
  working: boolean;
  /** The account's own rate-limit verdict: allowed, allowed_warning, rejected. */
  limit?: string;
  /** Questions it is blocked on. Nonzero means it cannot go on without you. */
  waiting: number;
  /** Messages written to the session that it has not read back. */
  unread: number;
  /** How long it has been failing to read them, in seconds.
   *
   *  Present only when the runner is prepared to call the session deaf — see
   *  `session::Session::deaf`. A number here means a restart is the only known
   *  cure, and that `revive` is what performs it. */
  deaf?: number;
  /**
   * Slash commands written while a turn was running, oldest first, waiting for
   * it to end.
   *
   * ⚠ **A command sent mid-turn does not run** — the CLI parks it and hands it
   * to the model as words, so `/rename` got a polite "nothing for me to do" and
   * no name was ever written. The runner holds it instead and sends it when the
   * turn ends; this is what says so on screen, and cancelling goes by the exact
   * text. Absent when nothing is waiting. See `session::State::held`.
   */
  held?: string[];
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
   * WHICH background calls are running. Mirrors `session::Summary::running`.
   *
   * ⚠ **Beside `background`, not instead of it.** The list ranks a row on
   * whether anything is running and never draws these; the session strip draws
   * these because a bare *1* is only a reason to ask (#740).
   */
  running?: Called[];
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
  /**
   * Who is holding what. Mirrors `tasks::Sweep`.
   *
   * ⚠ **Keyed like the sentences, and for the same reason.** The list draws the
   * transcripts on disk beside the running sessions, and a conversation that is
   * not running still has the list it kept — so this covers rows that have no
   * summary to hang a field on.
   */
  tasks?: Sweep;
}

/** Who is holding what, in one answer. Mirrors `tasks::Sweep`. */
export interface Sweep {
  /** By session id, for the cards. */
  readonly sessions: Record<string, TaskCount>;
  /** The holders who are not conversations, in the service's own order. */
  readonly elsewhere: readonly Held[];
}

/**
 * Somebody holding tasks who is not one of the console's conversations.
 * Mirrors `tasks::Held`.
 *
 * The unassigned pile is one of these, and it is the row nothing else on the
 * page can show: it belongs to no session, so it appears on no card.
 */
export interface Held {
  /** The service's own word — `Pippijn`, `nobody`. */
  readonly name: string;
  readonly open: number;
  readonly total: number;
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
  /**
   * The windows belonging to one model rather than to the plan, named by the
   * model. Absent when the runner has heard of none — the CLI files these in a
   * `model_scoped` array whose contents are whatever Anthropic scopes today, so
   * nothing here knows the names in advance. Mirrors `usage::Scoped`.
   */
  models?: Scoped[];
}

/** One model's own allowance. */
export interface Scoped extends Window {
  /** The model's display name, verbatim from the CLI — 'Fable'. */
  model: string;
}

/** The event kinds the runner emits, as a value so the wire can be checked
 *  against it rather than trusted. Mirrors `protocol::Event`. */
export const KINDS = [
  'joined',
  'started',
  'prompt',
  'accepted',
  'command',
  'shown',
  'text',
  'tool',
  'tool_result',
  'turn',
  'limit',
  'busy',
  'background',
  'exited',
  'deaf',
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
  /**
   * `ask` only: the call being asked about — the `tool_use` id, which is the
   * `id` of the `tool` event the CLI sent a moment earlier.
   *
   * ⚠ **Without it one action draws two widgets.** The CLI announces the call
   * and then asks about it, so a client that cannot join them shows a tool row
   * AND a permission card for one Write — and the card between two calls breaks
   * the run they would otherwise fold into. Absent from one of the CLI's three
   * `can_use_tool` call sites, so an unattachable question must still draw. */
  call?: string;
  title?: string;
  allowed?: boolean;
  /** `answered` only: what was chosen. See `protocol::Event::Answered` for why
   *  it travels with the verdict rather than being kept by whoever chose. */
  reply?: Reply;
  /** `deaf` only: how many messages are sitting in the pipe unread. */
  readonly unread?: number;
  /** `deaf` only: how long they have gone untouched, in seconds. */
  readonly seconds?: number;
}

/** What the transcript is drawn from.
 *
 *  Not the same as an event: consecutive text deltas are one paragraph on
 *  screen, and a tool's result belongs with the call it answers rather than
 *  wherever it happened to arrive. */
export interface Entry {
  /** `day` is not a thing that happened — it is the date the entries after it
   *  fall on, put in by [[fold]] when the conversation crosses midnight. */
  kind: 'said' | 'asked' | 'shown' | 'tool' | 'turn' | 'note' | 'ask' | 'day';
  text: string;
  /** `shown` only: the file name of a picture that was sent to this session,
   *  which is what `ConsoleApi.pictureAt` turns into a URL. Never the bytes: the
   *  transcript line holds a megabyte of base64 and the runner deliberately does
   *  not pass it on. */
  picture?: string;
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
  /**
   * Tool entries only: the call had no result when the console picked this
   * conversation up, so its answer may never have been written.
   *
   * ⚠ **This is not a verdict and must never be shown as one.** The one case
   * observed had SUCCEEDED — a detached `home-manager switch` that ran fine and
   * then booted out the console, and the `claude` process that would have
   * recorded the result, before it could be written. So the work is neither ok
   * nor failed; what is missing is the record of it. Rendering it as either
   * would be an invention, in a place a person uses to decide whether to
   * interrupt a session.
   *
   * Set at the `joined` boundary and cleared the moment a result does arrive —
   * because a call genuinely in flight when the console re-seeds looks exactly
   * like a dead one from here, and the difference only shows up when its answer
   * lands a minute later.
   */
  unrecorded?: boolean;
  /**
   * `asked` entries only: the runner has the message but the session has not
   * read it yet.
   *
   * ⚠ **Not a delivery failure and must not read as one.** The write to the
   * session succeeded; the CLI parks input that arrives mid-turn and releases it
   * in batches, up to twelve minutes later. Shown because the alternative —
   * nothing on screen at all until it is read — is indistinguishable from a
   * message that never arrived, which is why the same sentence got sent three
   * times in one evening.
   */
  queued?: boolean;
  /**
   * `ask` entries only: the decision has been written to the session and the
   * session has not acted on it yet.
   *
   * ⚠ **The same lie as [[queued]], told in green.** `Answered` is pushed once
   * the bytes reach the pipe, and the card drew its verdict straight from that —
   * so a session that had stopped reading showed *answered*, in the colour that
   * means done, while it sat blocked on the identical question. `health` wore
   * one for thirty-one minutes on 2026-08-08 having already reported it could go
   * no further without the answer.
   *
   * Cleared by the session speaking — a tool call, its result, a word of text.
   * There is no dedicated receipt for a decision the way a prompt has its
   * replay, and none is needed: the question blocked the turn, so anything at
   * all afterwards means the answer was taken up.
   */
  settling?: boolean;
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

/**
 * One task on a session's own list.
 *
 * ⚠ **Not the console's idea of work — the session's.** These are written by the
 * tasks service, so the list is what this conversation is holding — the tasks
 * assigned to it, whoever filed them. The console reads and never writes: two
 * surfaces editing one list is how the two copies start disagreeing, and the
 * `task` CLI is the other one.
 *
 * ⚠ **`active_form` and `blocked_by` are gone**, and were declared here for a
 * while against a runner that never sent them. The service has neither.
 */
export interface Task {
  readonly id: string;
  readonly subject: string;
  /** `open`, `doing`, `done` or `dropped`, in the service's own words. A string
   *  rather than a union because a state this console has not heard of yet is
   *  news to show, not a parse failure — see `standingOf` in the tasks sheet. */
  readonly status: string;
  /** Whether there is prose behind it worth opening. */
  readonly detailed: boolean;
  /**
   * How urgent, in the service's own words — `P0` to `P4`, and **absent** on
   * almost every task there is.
   *
   * ⚠ **Absence is not a sixth level.** An unranked task sorts exactly where
   * `P2` does, so `P0` and `P1` rise above the untriaged while `P3` and `P4`
   * sink below it. Nothing drawn for an absent rank, therefore: no chip, no
   * reserved column, nothing that would put a mark on 98% of the list to say
   * that nobody has thought about it.
   *
   * ⚠ **Do not sort on it here.** The rows arrive in the service's order, which
   * is the only place the ordering is decided.
   *
   * A string rather than a union, like `status`: a level the service invents
   * later is news to draw, not a parse failure.
   */
  readonly priority?: string;
}

/**
 * How much of a session's list is left. Mirrors `tasks::Count`.
 *
 * ⚠ **Absent means never handed anything, not an empty list.** Most
 * conversations have never been given a task, and a row reading `0/0` would put
 * a tally on every card on the page. The runner leaves those out of the map
 * entirely — but keeps a conversation that finished its list, because `0/9` is
 * a different fact and the better one to see.
 *
 * ⚠ **`total` is what is assigned now, not what ever was.** A task handed to
 * another conversation leaves both halves, so finishing work moves the fraction
 * the right way. A denominator counting everything ever assigned would only
 * grow, which is why this was one number for a while.
 */
export interface TaskCount {
  readonly open: number;
  readonly total: number;
  /**
   * How many are still in the built-in store the service replaced — absent
   * when there are none, which is the state to be in.
   *
   * ⚠ **Not part of the fraction, and drawn as a problem.** Every file there is
   * re-sent to its session 1.75 times per message with its whole body, which is
   * why the lists moved. A number here means a conversation migrated without
   * deleting, or is still filing work into the store nothing reads.
   */
  readonly stray?: number;
}

/**
 * One `Bash` command read the way the index reads it. Mirrors `parse::Parsed`.
 *
 * ⚠ **This is a report, not an offer to run anything.** Everything here
 * describes a command that already ran; the runner parses text and executes
 * none of it.
 */
export interface Parsed {
  /** Why the grammar could not read it. 0.4% of the corpus's calls fail, and a
   *  view that showed those as "did nothing" would be the worst kind of wrong. */
  readonly error?: string;
  readonly steps: readonly Step[];
  /** Commands whose operation is not in the table. Usually the whole answer to
   *  "why did this attribute nothing". */
  readonly unread?: readonly { readonly name: string; readonly count: number }[];
  /** Commands that exist because a determinate loop was run out — so a reader
   *  counting steps against the text they wrote is not left puzzled. */
  readonly unrolled?: number;
  /** Scripts inside a wrapper the grammar could not read: a hole in the middle
   *  of a parse that otherwise worked. */
  readonly nested_unparsed?: number;
}

/** One command in a parse, with what was decided about it. */
export interface Step {
  /** How many wrappers enclose it. Drawn as indentation. */
  readonly depth: number;
  /** The machine it ran on, when it was not this one. */
  readonly host?: string;
  /** The words **as the shell would have run them** — expanded, with a loop's
   *  variable replaced by this iteration's value. Deliberately not the words as
   *  written: why a path came out as it did is usually not in the text. */
  readonly argv: readonly string[];
  readonly reached: 'always' | 'on-success' | 'sometimes';
  /** The subshells enclosing it, outermost first. Two sibling `( … )` groups
   *  differ here, which is the difference between one directory and two. */
  readonly scope?: readonly number[];
  /** What its relative paths resolved against. Absent when a `cd` the reader
   *  could not follow made it unknowable. */
  readonly cwd?: string;
  /** The operation in one word. */
  readonly kind: string;
  /** What the operation says that its paths cannot — a search's pattern, a
   *  transform's program, the name of a command nobody has taught this yet. */
  readonly says?: string;
  readonly uses?: readonly Used[];
}

/** One file a command used, and whether that use is a fact. */
export interface Used {
  readonly path: string;
  readonly write: boolean;
  /** What the *text* said had to hold. */
  readonly reached: 'always' | 'on-success' | 'sometimes';
  /** Whether the text's condition and the call's outcome together make it
   *  certain.
   *
   *  ⚠ **One-sided.** `false` means "cannot say", never "did not happen". */
  readonly certain: boolean;
  /** The machine it is on, for a use that is not local. */
  readonly host?: string;
}
