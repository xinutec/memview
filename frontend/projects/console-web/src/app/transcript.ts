import { unhandled } from './exhaustive';
import {
  type Asked,
  type Call,
  type Change,
  type Entry,
  type Timed,
  type ToolCall,
  asking,
} from './models';
import { launched } from './workflow';

/**
 * Fold one event into the transcript so far.
 *
 * Copy-on-write: the array and every entry the event changes are new objects,
 * and everything else is shared. The rows are signal inputs, which see a change
 * only by reference — a mutated entry would draw as it was.
 */
export function fold(entries: readonly Entry[], event: Timed): Entry[] {
  const out = SPOKE.has(event.kind) ? entries.map(settled) : [...entries];
  switch (event.kind) {
    case 'text': {
      // Past queued messages only. The CLI reads a message sent mid-turn when the
      // turn ends, so it interrupts nothing; ending the model's block there would
      // cut one message, and any list or code fence in it, in two. A tool call
      // does interrupt, and starts a new block.
      const carries = lastSaid(out);
      if (carries !== undefined) {
        const said = out[carries];
        if (said?.kind === 'said') out[carries] = { ...said, text: said.text + event.text };
      } else {
        add(out, { kind: 'said', text: event.text, at: at(event) });
      }
      break;
    }
    case 'accepted':
      add(out, { kind: 'asked', text: event.text, at: at(event), queued: true, id: event.id });
      break;
    case 'withdrawn': {
      // Taken back before it was read: it was never said.
      const gone = out.findIndex(
        (entry) => entry.kind === 'asked' && !!entry.queued && entry.id === event.id,
      );
      if (gone !== -1) out.splice(gone, 1);
      break;
    }
    case 'command':
      add(out, { kind: 'asked', text: event.text, at: at(event) });
      break;
    case 'prompt': {
      // The runner echoes a queued message when the session reads it: move it
      // to the end rather than showing it twice.
      // By id when it has one: two messages may say the same words.
      const queued = (entry: Entry): entry is Asked => entry.kind === 'asked' && !!entry.queued;
      const read =
        (event.id === undefined
          ? undefined
          : out.find((entry): entry is Asked => queued(entry) && entry.id === event.id)) ??
        out.find((entry): entry is Asked => queued(entry) && entry.text === event.text);
      if (!read) {
        add(out, { kind: 'asked', text: event.text, at: at(event), id: event.id });
        break;
      }
      out.splice(out.indexOf(read), 1);
      out.push({ ...read, queued: undefined, at: at(event) ?? read.at });
      break;
    }
    case 'shown':
      add(out, { kind: 'shown', pictures: event.names, text: event.text, at: at(event) });
      break;
    case 'tool':
      if (out.some((entry) => entry.kind === 'tool' && entry.call === event.id)) break;
      add(out, {
        kind: 'tool',
        call: event.id,
        tool: event.name,
        does: event.does,
        text: describe(event.name, event.does),
        change: changed(event.does),
        at: at(event),
      });
      break;
    case 'tool_result': {
      const call = callIn(out, event.id) ?? running(out);
      if (!call) break;
      out[out.indexOf(call)] = {
        ...call,
        ok: event.ok,
        unrecorded: undefined,
        detail: event.detail,
        cut: event.cut ?? undefined,
        head: event.detail.split('\n', 1)[0],
        picture: event.image && call.text.startsWith('/') ? call.text : undefined,
        launched: call.does?.kind === 'workflow' ? launched(event.detail) : undefined,
      };
      break;
    }
    case 'ask': {
      const questions = event.does.kind === 'question' ? event.does.questions : undefined;
      const called = event.call ? callIn(out, event.call) : undefined;
      if (called) {
        out[out.indexOf(called)] = {
          ...called,
          ask: event.id,
          questions,
          text: event.title ?? called.text,
        };
        break;
      }
      add(out, {
        kind: 'ask',
        ask: event.id,
        tool: event.tool,
        does: event.does,
        text: event.title ?? describe(event.tool, event.does),
        at: at(event),
        questions,
        change: changed(event.does),
      });
      break;
    }
    case 'answered': {
      const index = out.findIndex((entry) => asking(entry) && entry.ask === event.id);
      if (index < 0) break;
      const question = out[index];
      if (question && asking(question)) {
        out[index] = {
          ...question,
          allowed: event.allowed,
          settling: true,
          reply: event.reply ?? undefined,
        };
      }
      break;
    }
    case 'turn':
      add(out, {
        kind: 'turn',
        text: `${event.turns} requests · ${elapsed(event.duration_ms)}`,
        at: at(event),
      });
      break;
    case 'exited':
      add(out, {
        kind: 'note',
        text: event.code === 0 ? 'session ended' : `session ended (${event.code ?? 'killed'})`,
        at: at(event),
      });
      break;
    case 'trouble':
      add(out, { kind: 'note', text: event.detail, at: at(event) });
      break;
    case 'joined':
      if (event.restarted) {
        for (const [index, entry] of out.entries()) {
          if (entry.kind === 'tool' && entry.ok === undefined) {
            out[index] = { ...entry, unrecorded: true };
          }
        }
      }
      add(out, {
        kind: 'note',
        text: event.earlier
          ? `${count(event.earlier, 'earlier event')}, read from the transcript`
          : 'nothing earlier could be read from the transcript',
        at: at(event),
      });
      break;
    case 'compacted':
      add(out, {
        kind: 'note',
        text: 'conversation compacted — everything above was summarised',
        at: at(event),
      });
      break;
    case 'deaf':
      add(out, {
        kind: 'note',
        text: `not reading — ${count(event.unread, 'message')} written and untouched for ${elapsed(event.seconds * 1000)}`,
        at: at(event),
      });
      break;
    // Carried in the session's own state, not on the transcript. A refused sign-in
    // is on it already, in the CLI's own words.
    case 'signed_out':
    case 'edited':
    case 'diverged':
    case 'started':
    case 'context':
    case 'background':
    case 'limit':
    case 'busy':
      break;
    default:
      unhandled(event);
  }
  return out;
}

/**
 * Where the model's current block is, stepping back over messages it has not
 * read yet. `undefined` when the run has been ended by anything else — a tool
 * call, a result, a turn — all of which genuinely end a block.
 */
function lastSaid(entries: readonly Entry[]): number | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry?.kind === 'said') return i;
    if (entry?.kind === 'asked' && entry.queued) continue;
    return undefined;
  }
  return undefined;
}

/** Events that mean the session is moving, so a decision has been taken up. */
const SPOKE: ReadonlySet<Timed['kind']> = new Set([
  'text',
  'tool',
  'tool_result',
  'turn',
  'prompt',
]);

/** The tool call with this id, if it is on the transcript. */
function callIn(entries: readonly Entry[], call: string): ToolCall | undefined {
  return entries.find((entry): entry is ToolCall => entry.kind === 'tool' && entry.call === call);
}

/** The newest call still without a result, for a result that names no call. */
function running(entries: readonly Entry[]): ToolCall | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry?.kind === 'tool' && entry.ok === undefined) return entry;
  }
  return undefined;
}

function settled(entry: Entry): Entry {
  return asking(entry) && entry.settling ? { ...entry, settling: undefined } : entry;
}

function at(event: Timed): number | undefined {
  return event.at ?? undefined;
}

/** Push, with a day divider first when the day changed. */
function add(entries: Entry[], entry: Entry): void {
  const day = dayOf(entry.at);
  if (day !== undefined && day !== dayOf(entries[entries.length - 1]?.at)) {
    entries.push({ kind: 'day', text: date(entry.at ?? 0), at: entry.at });
  }
  entries.push(entry);
}

function dayOf(at: number | undefined): string | undefined {
  if (at === undefined) return undefined;
  const when = new Date(at);
  return `${when.getFullYear()}-${when.getMonth()}-${when.getDate()}`;
}

function date(at: number): string {
  return new Date(at).toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
  });
}

/** What an `Edit` replaces; nothing for any other call. */
function changed(does: Call): Change | undefined {
  if (does.kind !== 'edit') return undefined;
  const { path, before, after, everywhere } = does;
  return { path, before, after, everywhere };
}

/** The one thing worth showing for a call. */
function describe(tool: string, does: Call): string {
  switch (does.kind) {
    case 'bash':
      return does.command;
    case 'edit':
      return does.path;
    case 'question':
      return does.questions[0]?.question ?? tool;
    case 'workflow':
      return does.name ?? tool;
    case 'other':
      return does.shown;
    default:
      unhandled(does);
      return tool;
  }
}

function count(n: number, thing: string): string {
  return `${n} ${thing}${n === 1 ? '' : 's'}`;
}

function elapsed(ms: number): string {
  if (!ms) return '0s';
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes < 60) return `${minutes}m ${seconds}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** The transcript grouped for drawing: runs of tool calls fold into one row. */
export type Block =
  { kind: 'one'; entry: Entry } | { kind: 'tools'; key: string; entries: ToolCall[] };

const A_RUN = 2;

export function blocks(entries: readonly Entry[]): Block[] {
  const out: Block[] = [];
  let run: ToolCall[] = [];
  const flush = () => {
    if (run.length >= A_RUN) {
      out.push({ kind: 'tools', key: run[0]?.call ?? `at-${out.length}`, entries: run });
    } else {
      for (const entry of run) out.push({ kind: 'one', entry });
    }
    run = [];
  };
  for (const entry of entries) {
    if (entry.kind === 'tool' && !(entry.ask !== undefined && entry.allowed === undefined)) {
      run.push(entry);
      continue;
    }
    flush();
    out.push({ kind: 'one', entry });
  }
  flush();
  return out;
}

export interface Ran {
  calls: number;
  failed: number;
  running: number;
  unrecorded: number;
  images: number;
}

export function ran(entries: readonly ToolCall[]): Ran {
  return {
    calls: entries.length,
    failed: entries.filter((entry) => entry.ok === false).length,
    running: entries.filter((entry) => entry.ok === undefined && !entry.unrecorded).length,
    unrecorded: entries.filter((entry) => entry.unrecorded).length,
    images: entries.filter((entry) => entry.picture).length,
  };
}
