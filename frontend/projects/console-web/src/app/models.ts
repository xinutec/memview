/**
 * The console's wire types, generated from the Rust definitions — see
 * `scripts/gen-types.sh` — plus the one shape the client builds for itself.
 */

import type { Question } from './questions';
import type { Event } from './generated/Event';
import type { Reply } from './generated/Reply';

export type { Called } from './generated/Called';
export type { Conversation } from './generated/Conversation';
export type { CorpusRead } from './generated/CorpusRead';
export type { Decision } from './generated/Decision';
export type { Described } from './generated/Described';
export type { Draft } from './generated/Draft';
export type { Event } from './generated/Event';
export type { Gist } from './generated/Gist';
export type { Holder } from './generated/Holder';
export type { Landmark } from './generated/Landmark';
export type { Line } from './generated/Line';
export type { Mark } from './generated/Mark';
export type { Message } from './generated/Message';
export type { Mode } from './generated/Mode';
export type { Overview } from './generated/Overview';
export type { Page } from './generated/Page';
export type { Parsed } from './generated/Parsed';
export type { Ranked } from './generated/Ranked';
export type { Reach } from './generated/Reach';
export type { Reading } from './generated/Reading';
export type { Renaming } from './generated/Renaming';
export type { Reply } from './generated/Reply';
export type { Scoped } from './generated/Scoped';
export type { Shown } from './generated/Shown';
export type { Start } from './generated/Start';
export type { Summary } from './generated/Summary';
export type { Sweep } from './generated/Sweep';
export type { Task } from './generated/Task';
export type { TaskCount } from './generated/TaskCount';
export type { Timed } from './generated/Timed';
export type { Used } from './generated/Used';
export type { Window } from './generated/Window';

export type Kind = Event['kind'];

/**
 * Every `kind` the runner can send, for narrowing a message at the boundary.
 * Checked both ways against the generated union: a variant added in Rust fails
 * `Unlisted`, a name removed there fails `satisfies`.
 */
export const KINDS = [
  'joined',
  'started',
  'accepted',
  'prompt',
  'command',
  'shown',
  'text',
  'context',
  'tool',
  'background',
  'tool_result',
  'turn',
  'limit',
  'busy',
  'ask',
  'answered',
  'compacted',
  'deaf',
  'exited',
  'trouble',
] as const satisfies readonly Kind[];

type Unlisted = Exclude<Kind, (typeof KINDS)[number]>;
const everyKindIsListed: Unlisted extends never ? true : never = true;
void everyKindIsListed;

/** One line of the transcript as drawn: what the fold makes of the events. */
export type Entry = Said | Asked | Sent | ToolCall | Asking | Noted;

interface Stamped {
  /** Epoch milliseconds, when the transcript says. */
  at?: number;
}

/** The model speaking; consecutive deltas are one entry. */
export interface Said extends Stamped {
  kind: 'said';
  text: string;
}

/** Something the person said or ran. `queued` until the runner reads it. */
export interface Asked extends Stamped {
  kind: 'asked';
  text: string;
  queued?: boolean;
}

/** A picture they sent, by the name the runner kept it under. */
export interface Sent extends Stamped {
  kind: 'shown';
  picture: string;
}

/** A question put to the person: a permission, or an AskUserQuestion. */
export interface Ask {
  /** The control request's id, which the answer carries back. */
  ask: string;
  /** Undefined until decided; then the verdict. */
  allowed?: boolean;
  /** Present when the tool was AskUserQuestion and its input could be read. */
  questions?: readonly Question[];
  reply?: Reply;
  /** Decided here, not yet taken up by the runner. */
  settling?: boolean;
}

/** A tool call. Carries the ask about it, when the CLI asked before running it. */
export interface ToolCall extends Stamped, Partial<Ask> {
  kind: 'tool';
  /** The `tool_use` id, which the result and the ask both name. */
  call?: string;
  tool: string;
  text: string;
  /** Undefined while running; then whether it succeeded. */
  ok?: boolean;
  detail?: string;
  /** The first line of `detail`. */
  head?: string;
  /** The full length, when `detail` was cut. */
  cut?: number;
  /** Was running when the runner restarted, so no result will come. */
  unrecorded?: boolean;
  /** Where the picture it returned lives, when it read one on this machine. */
  picture?: string;
}

/** An ask the fold could not attach to a call, drawn as its own entry. */
export interface Asking extends Stamped, Ask {
  kind: 'ask';
  tool: string;
  text: string;
}

/** A turn's cost, a note from the runner, or a day divider. */
export interface Noted extends Stamped {
  kind: 'turn' | 'note' | 'day';
  text: string;
}

/** An entry that is still, or was, a question — what the ask card draws. */
export type Questioned = (ToolCall & Ask) | Asking;

/**
 * Anything that names an ask: what everything keyed by one takes.
 *
 * Not the id itself. A `ToolCall` carries two ids — `call`, the tool_use,
 * and `ask`, the control request — and both are strings, so a function taking
 * `ask: string` accepts the wrong one happily and files an answer under an id no
 * runner will ever quote back. Taking the carrier makes that unsayable without a
 * branded type and the assertion minting one would need.
 */
export type Identified = Pick<Ask, 'ask'>;

export function asking(entry: Entry): entry is Questioned {
  return (entry.kind === 'tool' || entry.kind === 'ask') && entry.ask !== undefined;
}

/**
 * A question still open — the only thing that can be answered.
 *
 * A separate type because "already decided" was four runtime guards.
 * Every method that sends a verdict re-checked `allowed !== undefined` and
 * returned early, which is an invariant kept by remembering rather than by the
 * compiler: a fifth path would have answered a question twice. Taking this type
 * instead moves the check to the one place a `Questioned` becomes answerable.
 */
export type Unanswered = Questioned & { allowed?: undefined };

/** A question nobody has answered yet. */
export function pending(entry: Entry): entry is Unanswered {
  return asking(entry) && entry.allowed === undefined;
}
