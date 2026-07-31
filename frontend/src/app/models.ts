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

/** Session/project history mined from the transcripts — mirrors history.rs. */
export interface HistorySession {
  id: string;
  /** The name the session goes by ("home"), or its id when never named. */
  name: string;
  first: string;
  last: string;
  turns: number;
}

/** One session's share of one project — index into HistorySummary.sessions. */
export interface HistoryHand {
  session: number;
  turns: number;
  first: string;
  last: string;
}

export interface HistoryProject {
  name: string;
  turns: number;
  first: string;
  last: string;
  /** Who worked on it, most turns first. */
  hands: HistoryHand[];
  files: string[];
  /** Days with at least one turn. */
  days: string[];
}

export interface HistorySummary {
  generated: string;
  sessions: HistorySession[];
  projects: HistoryProject[];
  /** Total turns indexed; the turns themselves come from the search endpoint. */
  turns: number;
}

export interface HistoryHit {
  session: string;
  project: string | null;
  at: string;
  /** What was asked. */
  prompt: string;
  /** A window around the match in what Claude said; empty unless it matched. */
  reply: string;
  /** Which field matched: 'prompt', 'reply', or 'all' for an unfiltered list. */
  matched: string;
  /** BM25 score. Exposed so a ranking regression is visible, not just felt. */
  score: number;
}

/** How many matches a session or project holds, across the WHOLE match set. */
export interface HistoryTally {
  name: string;
  hits: number;
}

export interface HistorySearchResult {
  hits: HistoryHit[];
  /** Total matches, which may exceed hits.length. */
  total: number;
  /**
   * Where the whole match set lives, most hits first. For a broad term no page
   * of turns can answer "which session said this" — 973 matches for "backup"
   * span every session — but a count per session answers it in one line.
   */
  by_session: HistoryTally[];
  by_project: HistoryTally[];
}
