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
