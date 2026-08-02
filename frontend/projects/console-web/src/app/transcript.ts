import { Entry, SessionEvent } from './models';

/** Fold one event into the transcript.
 *
 *  The stream is finer-grained than anything worth reading. Text arrives a few
 *  words at a time, thinking arrives the same way and belongs in its own block,
 *  and a tool's verdict arrives after — sometimes long after — the call it
 *  answers. This is where that becomes a page: deltas of the same kind extend
 *  the block they are part of, and a result finds its call rather than landing
 *  at the bottom.
 *
 *  Returns the same array, extended. Kept as a plain function so the folding can
 *  be tested without a component, a browser or a running session. */
export function fold(entries: Entry[], event: SessionEvent): Entry[] {
  const last = entries[entries.length - 1];
  switch (event.kind) {
    case 'text':
      if (last?.kind === 'said') last.text += event.text ?? '';
      else entries.push({ kind: 'said', text: event.text ?? '' });
      break;
    case 'thinking':
      if (last?.kind === 'thought') last.text += event.text ?? '';
      else entries.push({ kind: 'thought', text: event.text ?? '' });
      break;
    case 'prompt':
      entries.push({ kind: 'asked', text: event.text ?? '' });
      break;
    case 'tool':
      entries.push({
        kind: 'tool',
        tool: event.name ?? 'tool',
        text: describe(event.name, event.input),
        // No verdict yet: `ok` stays undefined until the result arrives, which
        // is what the UI renders as "running".
      });
      break;
    case 'tool_result': {
      // Find the call this answers. Searched from the end because a session runs
      // several tools at once and the newest is nearly always the match.
      const call = [...entries].reverse().find((e) => e.kind === 'tool' && e.ok === undefined);
      if (call) call.ok = event.ok;
      break;
    }
    case 'ask':
      entries.push({
        kind: 'ask',
        ask: event.id,
        tool: event.tool ?? 'tool',
        // The CLI's own sentence when it offers one; it reads better than
        // anything reassembled from the arguments.
        text: event.title ?? describe(event.tool, event.input),
      });
      break;
    case 'answered': {
      const question = entries.find((e) => e.kind === 'ask' && e.ask === event.id);
      if (question) question.allowed = event.allowed;
      break;
    }
    case 'turn':
      entries.push({
        kind: 'turn',
        text: `${event.turns ?? 0} turn(s) · ${money(event.cost_usd)} · ${seconds(event.duration_ms)}`,
      });
      break;
    case 'exited':
      entries.push({
        kind: 'note',
        text: event.code === 0 ? 'session ended' : `session ended (${event.code ?? 'killed'})`,
      });
      break;
    case 'trouble':
      entries.push({ kind: 'note', text: event.detail ?? 'something went wrong' });
      break;
    // `started`, `busy` and `limit` are session state rather than transcript;
    // they belong in the header, and repeating them between paragraphs would
    // turn the page into a log.
    default:
      break;
  }
  return entries;
}

/** A tool call in one line: its most telling argument, not all of them. */
function describe(name: string | undefined, args: Record<string, unknown> | undefined): string {
  if (!args) return name ?? 'tool';
  for (const key of ['file_path', 'path', 'command', 'pattern', 'url', 'prompt', 'description']) {
    const value = args[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return Object.keys(args).join(', ');
}

function money(usd: number | undefined): string {
  if (!usd) return '$0.00';
  return usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

function seconds(ms: number | undefined): string {
  if (!ms) return '0s';
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}
