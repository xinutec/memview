import { Entry, SessionEvent } from './models';

/** Fold one event into the transcript.
 *
 *  The stream is finer-grained than anything worth reading. Text arrives a few
 *  words at a time, and a tool's verdict arrives after — sometimes long after — the call it
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
      else add(entries, { kind: 'said', text: event.text ?? '', at: event.at });
      break;
    case 'prompt':
      add(entries, { kind: 'asked', text: event.text ?? '', at: event.at });
      break;
    case 'tool':
      add(entries, {
        kind: 'tool',
        tool: event.name ?? 'tool',
        text: describe(event.name, event.input),
        at: event.at,
        // No verdict yet: `ok` stays undefined until the result arrives, which
        // is what the UI renders as "running".
      });
      break;
    case 'tool_result': {
      // Find the call this answers. Searched from the end because a session runs
      // several tools at once and the newest is nearly always the match.
      const call = [...entries].reverse().find((e) => e.kind === 'tool' && e.ok === undefined);
      if (!call) break;
      call.ok = event.ok;
      // What it said, not merely that it spoke. A tick with no answer under it
      // is the state this page was in: you could see that a search had run and
      // succeeded, and never what it found.
      call.detail = event.detail;
      call.cut = event.cut;
      // The first line, kept separately because it is what the row shows without
      // being asked. A result's answer is usually its first line — `3`, `done`,
      // `error: unknown flag` — so one line in the row answers the question most
      // of the time and costs no height at all, which on a phone is the whole
      // difference between a fact being there and being two taps away.
      call.head = (event.detail ?? '').split('\n', 1)[0];
      break;
    }
    case 'ask':
      add(entries, {
        kind: 'ask',
        ask: event.id,
        tool: event.tool ?? 'tool',
        // The CLI's own sentence when it offers one; it reads better than
        // anything reassembled from the arguments.
        text: event.title ?? describe(event.tool, event.input),
        at: event.at,
      });
      break;
    case 'answered': {
      const question = entries.find((e) => e.kind === 'ask' && e.ask === event.id);
      if (question) question.allowed = event.allowed;
      break;
    }
    case 'turn':
      add(entries, {
        kind: 'turn',
        // No cost here. It is not a bill (see budget.ts), and unlike the
        // header this line is built once from the event and never revisited —
        // so it could not hide itself again when the account is inside its
        // allowance, nor appear when it stops being. The gated total in the
        // header is where the number belongs.
        // "replies", because that is what the number counts: the assistant
        // messages this one exchange took. Measured — two exchanges reported 5
        // and 8, and their transcripts hold exactly 5 and 8 assistant messages.
        // Calling it "turns" alongside a header counting exchanges made one
        // conversation look like two different lengths.
        text: `${event.turns ?? 0} replies · ${seconds(event.duration_ms)}`,
        at: event.at,
      });
      break;
    case 'exited':
      add(entries, {
        kind: 'note',
        text: event.code === 0 ? 'session ended' : `session ended (${event.code ?? 'killed'})`,
        at: event.at,
      });
      break;
    case 'trouble':
      add(entries, { kind: 'note', text: event.detail ?? 'something went wrong', at: event.at });
      break;
    // Marked rather than blended. What is above came from the transcript on
    // disk; what is below, this console watched happen. They are both true and
    // they are not the same warranty — and without a line between them, a
    // resumed conversation reads as though the console had been there all along.
    case 'joined':
      add(entries, {
        kind: 'note',
        text: event.earlier
          ? `${event.earlier} earlier events, read from the transcript`
          : 'nothing earlier could be read from the transcript',
        at: event.at,
      });
      break;
    // Where the session stopped remembering. Worth a line of its own: the
    // messages above it are still on screen but are no longer in the session's
    // head, and it is where the exchange count in the header starts again.
    case 'compacted':
      add(entries, {
        kind: 'note',
        text: 'conversation compacted — everything above was summarised',
        at: event.at,
      });
      break;
    // `started`, `busy` and `limit` are session state rather than transcript;
    // they belong in the header, and repeating them between paragraphs would
    // turn the page into a log.
    default:
      break;
  }
  return entries;
}

/**
 * Append an entry, putting the date in front of it when the day has changed.
 *
 * ⚠ **Relative to the transcript, never to the clock.** The obvious alternative
 * — label anything from today with a time and everything else with a date — has
 * to ask what day it is now, and the answer changes while the page is open: a
 * message shown as `23:58` is still shown as `23:58` an hour later, when it means
 * yesterday. Comparing each entry to the one before it is a question with a
 * permanent answer.
 *
 * Nothing is inserted for an entry that carries no time, because a transcript
 * line is entitled not to say when it happened and an invented date is worse
 * than none.
 */
function add(entries: Entry[], entry: Entry): void {
  const day = dayOf(entry.at);
  if (day !== undefined && day !== dayOf(entries[entries.length - 1]?.at)) {
    entries.push({ kind: 'day', text: date(entry.at ?? 0), at: entry.at });
  }
  entries.push(entry);
}

/** Which calendar day a moment falls on, in the reader's own timezone. */
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

/** A tool call in one line: its most telling argument, not all of them. */
function describe(name: string | undefined, args: Record<string, unknown> | undefined): string {
  if (!args) return name ?? 'tool';
  for (const key of ['file_path', 'path', 'command', 'pattern', 'url', 'prompt', 'description']) {
    const value = args[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return Object.keys(args).join(', ');
}

function seconds(ms: number | undefined): string {
  if (!ms) return '0s';
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}
