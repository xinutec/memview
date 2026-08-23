/** API shapes — mirror the Rust serde structs. */

export interface Me {
  user_id?: string;
  display_name?: string;
  shared: boolean;
  auth_enabled: boolean;
}

export interface MemoryMeta {
  name: string;
  description: string;
  /** user | feedback | project | reference (free-form fallback). */
  mtype: string;
  modified: string | null;
}

/**
 * Which session wrote a memory, from its `originSessionId` frontmatter, and
 * the agent that session belongs to.
 *
 * Owner-only: the server omits it entirely for a share-link recipient, so its
 * presence is the permission check — there is nothing for the template to gate.
 */
export interface Origin {
  session: string;
  /** Absent when the session's transcript has been pruned since. */
  agent?: string | null;
}

export interface MemoryPage extends MemoryMeta {
  html: string;
  backlinks: MemoryMeta[];
  outlinks: MemoryMeta[];
  /** Wikilink targets not written yet. */
  dangling: string[];
  /** Owner-only; absent for a share-link recipient. */
  origin?: Origin;
}

export interface IndexPage {
  html: string;
  count: number;
}

export interface SearchHit extends MemoryMeta {
  snippet: string | null;
  /** BM25 score. Exposed so a ranking regression is visible, not just felt. */
  score: number;
}

/** What a search found — and whether it had to widen the question to find it. */
export interface SearchResult {
  hits: SearchHit[];
  /**
   * True when nothing matched every word and the query fell back to "any word".
   * Shown to the reader rather than swallowed: results for a question they did
   * not ask, presented as though they had, is the worst kind of quiet failure.
   */
  relaxed: boolean;
}

export interface GraphNode extends MemoryMeta {
  /** MEMORY.md `## section`, or null when the index links it under no heading. */
  section: string | null;
  /** Body length in bytes; ~50x range across the corpus, so scale it log-wise. */
  size: number;
  in_degree: number;
  out_degree: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  /**
   * What the link claims — `part-of`, `governs`, `because`, `refines`,
   * `supersedes`, `contradicts` — or null for a plain mention.
   */
  relation: string | null;
}

/** Signs of life for one memory — mirrors couse.rs. */
export interface Usage {
  /** Distinct sessions that mentioned it. */
  sessions: number;
  /** Turns that mentioned it. */
  turns: number;
  /** Times deliberately opened. */
  reads: number;
  /** Times written or edited. */
  edits: number;
  /** Most recent mention, ISO-8601, or null if never seen. */
  last: string | null;
  /**
   * Mentions per project, from the working directory of the line that named it.
   *
   * The context a memory is actually consulted in, which its own text does not
   * say — a rule filed under "rules" may in practice be a health-sync rule.
   * Empty for a memory never seen in a project directory, and empty for every
   * memory when the artefact predates this field — never absent, because the
   * server fills it in either way.
   */
  projects: Record<string, number>;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Section titles in MEMORY.md order — the legend's order. */
  sections: string[];
  /** Usage by memory name. Empty when no co-use artefact was mined. */
  usage: Record<string, Usage>;
  /** Pairs the work keeps using together, whether or not either links the other. */
  affinities: Affinity[];
}

/** A pull between two memories that the corpus never wrote down. */
export interface Affinity {
  a: string;
  b: string;
  /** Turns in which both were used. */
  turns: number;
  /** Distinct sessions those turns came from — the support. */
  sessions: number;
  /** Normalised mutual information, ~0..1. */
  npmi: number;
}

/** One thing that happened in the client — mirrors routes/telemetry.rs. */
export interface TelemetryEvent {
  /** `nav` for a route change, `tap` for a control. */
  kind: string;
  path: string;
  /** The control's visible text, verbatim; absent for a navigation. */
  label: string | null;
  /** The client's clock, epoch milliseconds — a batch arrives all at once, so
   *  the server's receive time cannot order the events inside it. */
  at: number;
}

export interface ShareInfo {
  active: boolean;
  token?: string;
  url?: string | null;
  created_at?: string;
  last_accessed_at?: string | null;
}

/**
 * How one agent uses one memory — mirrors agents.rs.
 *
 * Deliberate opens and edits of the file, not mentions of its name: a name is
 * mostly re-injected context, and counting those measures what recall pushed
 * into a session rather than what anyone worked with.
 */
export interface MemoryUse {
  reads: number;
  edits: number;
  /**
   * Uses a command *may* have made — a weaker claim, kept apart.
   *
   * A shell command after `&&` runs only if what preceded it succeeded, and one
   * exit status for a whole script often cannot say whether it did. Counting
   * those as fact overstates the record and dropping them understates it, so
   * they are neither: they are their own kind of evidence. Absent from the
   * artefact when zero, which is every tool-call path — an `Edit` either
   * replaced the text or changed nothing, and its result says which.
   */
  maybe_reads?: number;
  maybe_edits?: number;
}

/**
 * Who has been changing the files a query names — mirrors agents.rs.
 *
 * The companion to a memory search: that one asks what was written down, this
 * asks what was worked on, and a subtree nobody documented still has somebody
 * who knows it.
 */
export interface WorkMatch {
  name: string;
  /** Writes and edits across the matching files — what the ranking is on. */
  edits: number;
  /** Reads across the same files. Beside `edits`, never added to it. */
  reads: number;
  /** Lines committed across the matching files — size, where the counts above are frequency. */
  added: number;
  deleted: number;
  /**
   * File changes committed, NOT commits: one commit touching four matching
   * files counts four. Named for what it measures.
   */
  file_commits: number;
  /**
   * Machines this row's evidence touches, other than this one — so a total that
   * includes another host says so before the files are opened.
   */
  hosts: string[];
  /** The matching files, heaviest first: the evidence for the row. */
  files: WorkFile[];
}

/**
 * What one agent's commits did to one file. Added and deleted stay apart: a
 * rewrite removing 181 lines and adding 594 is not the same work as writing 413
 * from nothing, and deletion is work too.
 */
export interface LineDelta {
  added: number;
  deleted: number;
  commits: number;
}

/** One file a query matched, and how one agent used it. */
export interface WorkFile {
  path: string;
  /**
   * The names this file used to have, newest last. Absent for the ordinary
   * case; present, it is why a file created last week can carry a year of
   * history — and unsaid, that history reads as a counting bug.
   */
  was?: string[];
  /** Every use, tool call and shell command together. */
  reads: number;
  edits: number;
  /**
   * How much of the above came from a `Bash` call rather than `Write`/`Edit` —
   * the shell, the Python inside a heredoc, and the other machines' shells
   * alike. A file with changes and no tool edits is somebody working through
   * `sed` or `python3 -`, not a mistake — without the split there is no way to
   * see which.
   */
  shell_reads: number;
  shell_edits: number;
  /**
   * The machine this file is on, absent when it is this one. Remote use can
   * only come from an `ssh` payload, so `shell_*` already says where the
   * numbers came from and this says where the *file* is.
   */
  host?: string | null;
  /**
   * Lines this agent committed to the file, and in how many commits. The same
   * work measured a second way — never added to the counts above, which would
   * count it twice.
   */
  added: number;
  deleted: number;
  commits: number;
}

/** One named session and where its work landed — mirrors agents.rs. */
export interface Agent {
  name: string;
  /** Main-loop transcripts under this name; more than one when resumed. */
  transcripts: number;
  /**
   * Transcripts of subagents and workflow agents this session dispatched.
   * Their work counts as this agent's — it asked for it.
   */
  delegated: number;
  /**
   * The session ids filed under this name — the join to the corpus, where each
   * memory records the `originSessionId` that wrote it.
   */
  sessions: string[];
  /**
   * Every file touched under the code root, keyed by path relative to it.
   * `reads`/`writes` keep only the repository; this keeps the subtree, which is
   * what makes "who works on the Dhall configs" a question with an answer.
   */
  paths: Record<string, MemoryUse>;
  /**
   * The same, for files used by shell commands rather than tool calls — a
   * `sed -i`, a `cp`, a `>` redirect. Kept apart from `paths` so the figures
   * that were always there go on meaning what they meant, and unioned with it
   * only when a query asks who works on something.
   */
  shell_paths: Record<string, MemoryUse>;
  /**
   * Files used on other machines, keyed `host:/absolute/path`. Entirely from
   * `ssh`/`kubectl exec` payloads; git cannot attribute these, because the
   * commits are made over there.
   */
  remote_paths: Record<string, MemoryUse>;
  /**
   * Lines committed per path, attributed by the earliest mention of the commit
   * hash — the only join available when every commit shares one git author.
   */
  commit_lines: Record<string, LineDelta>;
  /** Commits attributed to this agent, across every repository. */
  commits: number;
  /** Files opened, per project directory. Lifetime totals, undecayed. */
  reads: Record<string, number>;
  /** Files written or edited, per project directory. Lifetime, undecayed. */
  writes: Record<string, number>;
  /**
   * Which memories this agent works with, keyed by memory name. The companion
   * question to `reads`/`writes`: those say where it is responsible, this says
   * what it has consulted.
   */
  memories: Record<string, MemoryUse>;
  /**
   * Recency-weighted days present, per project — what the ordering uses.
   * Days rather than files, so one busy afternoon cannot outvote a fortnight.
   */
  recent_reads: Record<string, number>;
  recent_writes: Record<string, number>;
  first: string;
  last: string;
}

export interface AgentsResult {
  generated: string;
  /** Where each renamed file ended up, old name to current. */
  renames: Record<string, string>;
  /**
   * Commits found under the code root, and how many no transcript mentions.
   * Reported rather than dropped: Claude Code prunes old sessions, so anything
   * predating the corpus has nobody left to credit — and a reader comparing
   * these lines against `git log` needs to know the coverage first.
   */
  commits: number;
  unattributed: number;
  agents: Agent[];
}

/**
 * What a turn's work turned out to be. Mirrors `reader::doing::Verdict`.
 *
 * ⚠ **`unknown` is a real state, not a synonym for `ok`.** An interruption is
 * not a result at all but a separate message, so the call it stopped never gets
 * an answer — collapsing it into either would invent one. The Rust doc says
 * exactly this; the word on the wire is `unknown`, and mirroring it here as
 * `'unrecorded'` from the prose is what dev-lint caught.
 */
export type Verdict = 'unknown' | 'ok' | 'failed' | 'rejected';

/** One minute of one session's work. Mirrors `api::Moment`. */
export interface Moment {
  /** Unix minute, the key this row is opened by — with `agent`. */
  at: number;
  agent: string;
  /** Absent for work that belongs to no project directory. */
  project?: string | null;
  /** Set when the work happened on another machine, over `ssh`/`kubectl`. */
  host?: string | null;
  kind: string;
  /** How many activities this minute folded into one row. */
  n: number;
  /**
   * Index into `Timeline.episodes` — which instruction this was part of.
   *
   * Absent for work with no prompt above it in its transcript, which is what a
   * resumed session looks like from the outside.
   */
  episode?: number | null;
  verdict: Verdict;
  /**
   * How many effects opening this row would show.
   *
   * ⚠ **12.6% of rows are 0**, measured over the live artefact — 58,644 of
   * 466,951, and 27 of the newest 200. A `test` or `build` minute need not
   * touch a file. Drawn on the row so a tap is never spent learning there was
   * nothing to learn, and so a 936-effect turn announces itself first.
   */
  effects: number;
}

/** A page of the timeline, and the shape of everything the filter matched. */
/** One instruction, and the stretch of work carried out under it. */
export interface TimelineEpisode {
  agent: string;
  /**
   * The minute of its FIRST row, which may be older than anything on this page:
   * an episode says how large the stretch was, not how much of it fitted.
   */
  at: number;
  until: number;
  n: number;
}

export interface Timeline {
  moments: Moment[];
  /**
   * The instructions the moments were carried out under, referenced by
   * `Moment.episode`. Only those this page touches.
   */
  episodes: TimelineEpisode[];
  /**
   * Kinds of work across the WHOLE filtered range, biggest first — `[kind, n]`.
   * Two hundred rows cannot show the shape of two hundred thousand, so the
   * server counts it and the page draws it beside them.
   */
  summary: [string, number][];
  total: number;
  failed: number;
}

/**
 * What one effect did. Mirrors `reader::effects::Did`.
 *
 * ⚠ **One letter, because that is what the wire carries.** The artefact holds
 * hundreds of thousands of these and is read over a VPN, so every variant is
 * `#[serde(rename)]`d to a character. Spelling them out here as `'read' |
 * 'wrote' | …` type-checked, rendered, and passed its own test — because the
 * fixture had been written from the same wrong assumption. `dev-lint`'s
 * wire-mirror check is what caught it.
 */
export type Did =
  /** Opened and read. */
  | 'r'
  /** Changed: a `>` redirect, a `sed -i`, the destination of a `cp`, an `rm`. */
  | 'w'
  /**
   * Searched *in*. Kept apart from a plain read because "who grepped for this"
   * and "who read this" are different questions.
   */
  | 's'
  /** Named a subject the text does not determine — see `Evidence.unnamed`. */
  | 'u';

/** One thing a turn did, and the command that did it. Mirrors `api::Effect`. */
export interface Effect {
  at: number;
  agent: string;
  did: Did;
  /** Absent when the subject could not be named — see `Evidence.unnamed`. */
  path?: string | null;
  /** A glob or search pattern, where the subject was a set and not a file. */
  pattern?: string | null;
  host?: string | null;
  /** Verbatim. Owner-only for this reason, and never behind a share token. */
  command: string;
  /** Whether the command certainly ran, or only may have — `a && b`. */
  reached: boolean;
  verdict: Verdict;
}

/** What a turn did, keyed by the `(agent, at)` a timeline row already carries. */
export interface Evidence {
  effects: Effect[];
  total: number;
  /**
   * Effects whose subject nobody could name.
   *
   * ⚠ **Drawn, never dropped.** 7,305 of these exist in the artefact, and a
   * panel showing only what resolved would read as a complete account of the
   * turn. Saying "and 12 more this could not name" is the difference between
   * evidence and a summary.
   */
  unnamed: number;
}

/** One row of a ranked table in the corpus survey. */
export interface Ranked {
  name: string;
  n: number;
}

/** A path, command or host with both directions. */
export interface Both {
  name: string;
  reads: number;
  writes: number;
}

/**
 * What the reader makes of every shell command the fleet has run.
 *
 * ⚠ **Mined nightly, not computed on request.** The survey takes 13 seconds over
 * 146k commands, so `corpus_at` is the age of the answer and the page says so —
 * a figure here and a figure from `--bin shell-files` can differ by a night and
 * by nothing else.
 *
 * ⚠ **Every list is truncated; no total is.** The counts above each list are
 * over everything, and the lists are the top of it. A summary whose total was
 * the length of its own list is the failure this shape is built to avoid.
 */
export interface CorpusRead {
  /** When the corpus was last written, epoch seconds. */
  corpus_at: number | null;
  calls: number;
  unparsed: number;
  /** Commands *run* — a determinate loop is unrolled before this counts it. */
  commands: number;
  unrolled: number;
  handled: number;
  unhandled: number;
  /** `handled` over `commands`, computed server-side so two clients cannot
   *  round it two ways. */
  understood: number;
  reads: number;
  writes: number;
  distinct_paths: number;
  /** File uses by what had to hold for the command naming them to run. */
  always: number;
  on_success: number;
  sometimes: number;
  /** Those an outcome confirms actually happened. */
  certain: number;
  /**
   * Uses whose subject the text does not determine — `$f` bound by a loop the
   * transcript never shows.
   *
   * ⚠ **This is the honest ceiling, and it is drawn beside the coverage rather
   * than below it.** It is a property of the corpus, not of the reader: a name
   * fed in at runtime has no answer in the text, and inventing one would be
   * worse than counting it unknown.
   */
  unnamed: number;
  opaque: number;
  unnamed_by_word: number;
  unnamed_bounded: number;
  unnamed_computed: number;
  refused_here: number;
  /**
   * Tables the fleet's SQL read and changed.
   *
   * ⚠ **Never added to `reads`/`writes`.** A table is not a file, and measured
   * over this corpus SQL names a file exactly never — no `INTO OUTFILE`, no
   * `LOAD DATA INFILE`, no sqlite `.read`. Folding them together would inflate
   * the file figure with subjects that have no path.
   */
  table_reads: number;
  table_writes: number;
  distinct_tables: number;
  tables: Both[];
  sql: Ranked[];
  doing: Ranked[];
  renames: number;
  busiest: Both[];
  writers: Both[];
  hosts: Both[];
  unread: Ranked[];
  opaque_words: Ranked[];
}
