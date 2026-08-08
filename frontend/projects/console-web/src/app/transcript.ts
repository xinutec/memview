import { Entry, SessionEvent } from './models';
import { QUESTION_TOOL, questionsOf } from './questions';

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
  // The session speaking is the receipt for every decision written to it. A
  // question blocks the turn, so a tool call, a result or a word of text can only
  // mean the answer was read and acted on — see [[Entry.settling]]. Ahead of the
  // switch because it is true of the whole page rather than of one entry, and
  // several kinds of event carry it.
  if (SPOKE.has(event.kind)) {
    for (const entry of entries) if (entry.settling) entry.settling = undefined;
  }
  switch (event.kind) {
    case 'text':
      if (last?.kind === 'said') last.text += event.text ?? '';
      else add(entries, { kind: 'said', text: event.text ?? '', at: event.at });
      break;
    // Written to the session, not yet read by it. On screen immediately, marked
    // — see [[Entry.queued]] for why the wait must be visible.
    case 'accepted':
      add(entries, { kind: 'asked', text: event.text ?? '', at: event.at, queued: true });
      break;
    // A slash command, sent or read back off the transcript — and deliberately
    // NOT marked, unlike every other message this console writes.
    //
    // ⚠ **A command has no read receipt and cannot be given one.** Measured
    // 2026-08-08: `--replay-user-messages` replays a prompt and does not replay a
    // command, so the event that clears the marker never arrives. Marking one
    // made `life` show *waiting to be read* right through the compaction it had
    // already begun — a claim the console had no way left to withdraw.
    case 'command':
      add(entries, { kind: 'asked', text: event.text ?? '', at: event.at });
      break;
    case 'prompt': {
      // ⚠ **The echo promotes the waiting entry rather than adding a second
      // one.** The two events describe one message — the runner taking it and
      // the CLI reading it — and appending would show every message twice.
      //
      // The FIRST match, by text: stdin is a queue and the echo comes back in
      // the order it was written, so the oldest waiting copy is the one this
      // answers. Matching the newest would leave the oldest waiting for ever
      // whenever the same words are sent twice — which is exactly what somebody
      // does when they think the first one failed.
      const at = entries.findIndex(
        (entry) => entry.kind === 'asked' && entry.queued && entry.text === (event.text ?? ''),
      );
      if (at < 0) {
        add(entries, { kind: 'asked', text: event.text ?? '', at: event.at });
        break;
      }
      // ⚠ **Moved to the end, not cleared where it sits.** The entry went in when
      // the RUNNER took the message, and the CLI may not read it for minutes —
      // twelve, measured. Everything the session did in between was appended
      // below it, so the transcript read as though the message had been seen and
      // then the work continued, when all of it predates the session reading a
      // word (memview #117).
      //
      // Where it waited is the sender's own timeline and is why it is shown at
      // once; where it lands is where it enters the conversation. The jump is the
      // information: above it is what happened before it was read, below it is
      // what happened after.
      const [read] = entries.splice(at, 1);
      read.queued = undefined;
      // Stamped when it was read, so the clock down the page stays monotonic —
      // the position and the time have to agree or the day markers lie.
      read.at = event.at ?? read.at;
      entries.push(read);
      break;
    }
    // A picture that was sent to this session. Its own entry rather than an
    // `asked` carrying an image, because the two are separately optional: a
    // screenshot with nothing said is the commonest message this carries, and
    // words about a picture arrive as an ordinary `prompt` straight after.
    case 'shown':
      add(entries, { kind: 'shown', text: '', picture: event.name, at: event.at });
      break;
    case 'tool':
      // ⚠ **The same call can arrive twice, and did.** An upgrade re-seeds from
      // the transcript while the bytes still in the child's pipe are drained by
      // the new image — and a line can be in both. Measured on this console:
      // `toolu_01XMy9…` seeded at 15:21:58 and pushed again live at 15:22:25,
      // either side of the `joined` that marks the boundary.
      //
      // Dropping the second copy rather than showing it, because the copies do
      // not share a fate: only one result arrives, so the other row would sit at
      // "running" for ever — which is exactly what a genuinely blocked session
      // looks like, and the last thing this page should say by accident.
      if (event.id && entries.some((e) => e.kind === 'tool' && e.call === event.id)) break;
      add(entries, {
        kind: 'tool',
        call: event.id,
        tool: event.name ?? 'tool',
        text: describe(event.name, event.input),
        at: event.at,
        // No verdict yet: `ok` stays undefined until the result arrives, which
        // is what the UI renders as "running".
      });
      break;
    case 'tool_result': {
      // ⚠ **By id, not by recency.** This used to take the newest call with no
      // verdict, which is right only while one tool runs at a time — and the
      // runner reports background tasks that finish long after the calls made
      // since. The id is on both events and has been all along.
      //
      // The old rule is kept for an event that carries no id, which is what a
      // transcript line from before this can be.
      const named = event.id
        ? entries.find((e) => e.kind === 'tool' && e.call === event.id)
        : undefined;
      const call =
        named ?? [...entries].reverse().find((e) => e.kind === 'tool' && e.ok === undefined);
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
        // Only for the tool that asks a person something. Undefined for every
        // other, which is what leaves them with allow and refuse.
        questions: event.tool === QUESTION_TOOL ? questionsOf(event.input) : undefined,
      });
      break;
    case 'answered': {
      const question = entries.find((e) => e.kind === 'ask' && e.ask === event.id);
      if (question) {
        question.allowed = event.allowed;
        // Written to the pipe, not yet acted on — see [[Entry.settling]]. The
        // card takes the answer immediately, because the tap must land somewhere
        // at once, but it does not yet claim the session has it.
        question.settling = true;
        // What was chosen, which only the runner knows for every client — see
        // `protocol::Event::Answered`. Absent for every tool that is not a
        // question, and for a refusal.
        question.reply = event.reply;
      }
      break;
    }
    case 'turn':
      add(entries, {
        kind: 'turn',
        // No cost here. It is not a bill — the session runs on the subscription
        // — and a figure in dollars between two paragraphs reads as one. The
        // session's total is on the details sheet, labelled; how much room is
        // left is the utilisation strip's question.
        // ⚠ **"requests", because that is what the number counts** — round
        // trips to the model, not messages anybody sees. It said "turns" first,
        // which clashed with a header counting exchanges, and then "replies",
        // which is wrong in a way that only shows on a long answer: measured on
        // one exchange of this console's own transcript, the event reported 54
        // against 83 assistant messages and 53 tool calls. Fifty-three of those
        // requests existed because a tool had to run and be read; the
        // fifty-fourth wrote the answer. On a short exchange the two counts
        // coincide, which is how "replies" survived.
        text: `${event.turns ?? 0} requests · ${elapsed(event.duration_ms)}`,
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
    // The session has stopped reading its stdin. In the transcript and not only
    // in the header, because it is a thing that happened at a moment — it
    // belongs *after* the message it failed to read, where somebody scrolling
    // back can see which message went unanswered and when.
    case 'deaf':
      add(entries, {
        kind: 'note',
        text: `not reading — ${count(event.unread, 'message')} written and untouched for ${elapsed((event.seconds ?? 0) * 1000)}`,
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

/**
 * How long something took, at the scale it took it.
 *
 * ⚠ **A tenth of a second stops being information after a minute or two.** This
 * printed `1274.1s` for a turn that ran twenty-one minutes: a number nobody can
 * read at a glance and whose last digit describes a rounding error next to the
 * `cargo build` inside it. Minutes are what that scale wants, and the
 * sub-second precision is kept where it is the whole point — a call that either
 * returned at once or did not.
 */
/**
 * The events that can only come from the session's own process.
 *
 * ⚠ **`busy` is deliberately absent.** A status is announced only when it
 * changes (memview #112), so it can arrive from a session that then goes quiet
 * for half an hour — which is exactly the state this is meant to distinguish.
 * `exited` too: a session that died without reading the answer did not take it
 * up, and saying otherwise on its last card would be the same lie one last time.
 */
const SPOKE = new Set(['text', 'tool', 'tool_result', 'turn', 'prompt']);

/** `1 message`, `3 messages` — plural only when it should be. */
function count(many: number | undefined, thing: string): string {
  const n = many ?? 0;
  return `${n} ${thing}${n === 1 ? '' : 's'}`;
}

function elapsed(ms: number | undefined): string {
  if (!ms) return '0s';
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes < 60) return `${minutes}m ${seconds}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/**
 * A run of consecutive tool calls, or anything else on its own.
 *
 * ⚠ **Grouped for rendering, not for folding.** The entries themselves are left
 * exactly as [[fold]] built them — a result still finds its call by id, and
 * nothing downstream has to know a group exists. Doing it the other way, by
 * folding calls into a container entry, would have put that id lookup inside a
 * nested list for no gain.
 */
export type Block =
  { kind: 'one'; entry: Entry } | { kind: 'tools'; key: string; entries: Entry[] };

/**
 * How many consecutive calls it takes before they are worth folding away.
 *
 * Two, because two calls are 230px on a phone and their summary is one 48px row
 * — the saving is already most of a screen. One call is left alone: a group of
 * one costs a tap and saves nothing.
 */
const A_RUN = 2;

/**
 * Gather runs of tool calls so a transcript reads as what was said.
 *
 * A tool row is 115px at phone width and a turn can hold a dozen, so a
 * conversation with any work in it is mostly machinery — and the machinery is
 * what somebody scrolls *past* to find the answer. Folded, a run is one row that
 * says how many and whether any failed.
 *
 * Anything that is not a tool call breaks the run, which is what makes the
 * grouping follow the shape of the conversation rather than a count: a question,
 * an answer or a day marker between two calls means they were two separate
 * pieces of work.
 */
export function blocks(entries: readonly Entry[]): Block[] {
  const blocks: Block[] = [];
  let run: Entry[] = [];
  const flush = () => {
    if (run.length >= A_RUN) {
      // Keyed by the first call's id, which is stable across re-renders and
      // across a re-seed: what a reader opened stays open. A run whose first
      // entry carried no id falls back to its position, which is stable enough
      // for a transcript that only ever grows at the end.
      blocks.push({ kind: 'tools', key: run[0].call ?? `at-${blocks.length}`, entries: run });
    } else {
      for (const entry of run) blocks.push({ kind: 'one', entry });
    }
    run = [];
  };
  for (const entry of entries) {
    if (entry.kind === 'tool') {
      run.push(entry);
      continue;
    }
    flush();
    blocks.push({ kind: 'one', entry });
  }
  flush();
  return blocks;
}

/** What a folded run says about itself: how many, and how many went wrong. */
export function ran(entries: readonly Entry[]): { calls: number; failed: number; running: number } {
  return {
    calls: entries.length,
    failed: entries.filter((entry) => entry.ok === false).length,
    running: entries.filter((entry) => entry.ok === undefined).length,
  };
}
