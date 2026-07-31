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

export interface MemoryPage extends MemoryMeta {
  html: string;
  backlinks: MemoryMeta[];
  outlinks: MemoryMeta[];
  /** Wikilink targets not written yet. */
  dangling: string[];
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

/** One named session and where its work landed — mirrors agents.rs. */
export interface Agent {
  name: string;
  /** Transcripts under this name; more than one when a session was resumed. */
  transcripts: number;
  /** Files opened, per project directory. Lifetime totals, undecayed. */
  reads: Record<string, number>;
  /** Files written or edited, per project directory. Lifetime, undecayed. */
  writes: Record<string, number>;
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
  agents: Agent[];
}
