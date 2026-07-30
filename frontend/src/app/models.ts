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

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Section titles in MEMORY.md order — the legend's order. */
  sections: string[];
}

export interface ShareInfo {
  active: boolean;
  token?: string;
  url?: string | null;
  created_at?: string;
  last_accessed_at?: string | null;
}
