/** The console API's wire types. Mirrors `console/src/api.rs` and
 *  `console/src/protocol.rs`; the Rust side is the definition. */

export interface Summary {
  id: string;
  dir: string;
  /** Seconds since the epoch. */
  started: number;
  alive: boolean;
  model?: string;
  /** What the CLI last said it was doing, while it is doing anything. */
  busy?: string;
  turns: number;
  cost_usd: number;
  /** Questions it is blocked on. Nonzero means it cannot go on without you. */
  waiting: number;
  /** The first instruction, kept as the session's name. */
  asked?: string;
}

export interface Overview {
  dirs: string[];
  repos: string[];
  sessions: Summary[];
}

/** The event kinds the runner emits, as a value so the wire can be checked
 *  against it rather than trusted. Mirrors `protocol::Event`. */
export const KINDS = [
  'started',
  'prompt',
  'text',
  'thinking',
  'tool',
  'tool_result',
  'turn',
  'limit',
  'busy',
  'exited',
  'trouble',
  'ask',
  'answered',
] as const;

export type Kind = (typeof KINDS)[number];

/** One thing that happened in a session. The `kind` discriminates; every other
 *  field depends on it. */
export interface SessionEvent {
  kind: Kind;
  model?: string;
  cwd?: string;
  tools?: number;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  ok?: boolean;
  cost_usd?: number;
  turns?: number;
  duration_ms?: number;
  stop?: string;
  window?: string;
  status?: string;
  resets_at?: number;
  code?: number;
  detail?: string;
  /** `ask` only: the tool it wants to run, and the question's own id. */
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
  kind: 'said' | 'asked' | 'thought' | 'tool' | 'turn' | 'note' | 'ask';
  text: string;
  /** Tool entries only, once the result comes back. */
  ok?: boolean;
  tool?: string;
  /** `ask` entries only: the control-request id to answer with, and the verdict
   *  once there is one. Undecided is the state that needs a person. */
  ask?: string;
  allowed?: boolean;
}
