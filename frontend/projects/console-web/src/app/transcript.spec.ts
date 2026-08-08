import { describe, expect, it } from 'vitest';

import { Entry, SessionEvent } from './models';
import { blocks, fold, ran } from './transcript';

/** Fold a whole stream, the way the component does. */
function transcript(...events: SessionEvent[]): Entry[] {
  return events.reduce<Entry[]>((entries, event) => fold(entries, event), []);
}

describe('transcript', () => {
  it('joins the deltas of one answer into one paragraph', () => {
    // The wire delivers an answer a few words at a time. Rendered one event per
    // line, a two-sentence reply becomes twenty ragged fragments.
    const seen = transcript(
      { kind: 'text', text: 'I have ' },
      { kind: 'text', text: 'read the file' },
      { kind: 'text', text: ' and it is fine.' },
    );
    expect(seen).toEqual([{ kind: 'said', text: 'I have read the file and it is fine.' }]);
  });

  it('gives a tool result to the call it answers, not to the newest line', () => {
    // Results arrive after the call, and often after other calls have been made.
    // Appending them in arrival order would attach an error to the wrong tool —
    // the one thing on this page nobody may get wrong.
    const seen = transcript(
      { kind: 'tool', id: 'a', name: 'Read', input: { file_path: '/tmp/one' } },
      { kind: 'tool', id: 'b', name: 'Bash', input: { command: 'ls' } },
      { kind: 'tool_result', id: 'b', ok: false },
      { kind: 'tool_result', id: 'a', ok: true },
    );
    const tools = seen.filter((e) => e.kind === 'tool');
    expect(tools.map((t) => [t.tool, t.ok])).toEqual([
      ['Read', true],
      ['Bash', false],
    ]);
  });

  it('shows a call with no result yet as running', () => {
    const [tool] = transcript({
      kind: 'tool',
      id: 'a',
      name: 'Bash',
      input: { command: 'sleep 60' },
    });
    expect(tool.ok).toBeUndefined();
    expect(tool.text).toBe('sleep 60');
  });

  it('names a tool call by its most telling argument', () => {
    const [read] = transcript({
      kind: 'tool',
      name: 'Read',
      input: { file_path: '/etc/hosts', limit: 5 },
    });
    expect(read.text).toBe('/etc/hosts');
    const [task] = transcript({ kind: 'tool', name: 'Task', input: { subagent_type: 'Explore' } });
    expect(task.text).toBe('subagent_type');
  });

  it('leaves session state out of the transcript', () => {
    // These belong in the header. In the body they would break the reading with
    // a line every few seconds saying nothing new.
    expect(
      transcript(
        { kind: 'started', model: 'x', cwd: '/tmp', tools: 3 },
        { kind: 'busy', status: 'requesting' },
        { kind: 'limit', window: 'five_hour', status: 'allowed' },
      ),
    ).toEqual([]);
  });

  it('says when a session ended, and how', () => {
    expect(transcript({ kind: 'exited', code: 0 })[0].text).toBe('session ended');
    expect(transcript({ kind: 'exited' })[0].text).toContain('killed');
  });
});

describe('questions', () => {
  it('shows a question as undecided until it is answered', () => {
    // The undecided state is what the UI turns into a pair of buttons, so it has
    // to be distinguishable from both verdicts rather than defaulting to one.
    const [ask] = transcript({
      kind: 'ask',
      id: 'q1',
      tool: 'Bash',
      title: 'Claude wants to run rm -rf build',
      input: { command: 'rm -rf build' },
    });
    expect(ask.kind).toBe('ask');
    expect(ask.ask).toBe('q1');
    expect(ask.allowed).toBeUndefined();
    // The CLI's own sentence beats one reassembled from the arguments.
    expect(ask.text).toBe('Claude wants to run rm -rf build');
  });

  it('carries the options of a question through to the entry', () => {
    const [ask] = transcript({
      kind: 'ask',
      id: 'q1',
      tool: 'AskUserQuestion',
      input: {
        questions: [
          {
            question: 'which way?',
            header: 'Way',
            multiSelect: false,
            options: [
              { label: 'left', description: 'go left' },
              { label: 'right', description: 'go right' },
            ],
          },
        ],
      },
    });
    expect(ask.questions?.[0].options.map((o) => o.label)).toEqual(['left', 'right']);
  });

  it('leaves every other tool without options, which is what keeps allow and refuse', () => {
    // ⚠ The check is on the tool's *name*, not on whether the arguments happen to
    // parse. A tool of our own that took a `questions` argument would otherwise
    // be answered instead of approved.
    const [ask] = transcript({
      kind: 'ask',
      id: 'q1',
      tool: 'Bash',
      input: { questions: [{ question: 'which way?', options: [{ label: 'left' }] }] },
    });
    expect(ask.questions).toBeUndefined();
  });

  it('falls back to the arguments when the CLI offers no sentence', () => {
    const [ask] = transcript({
      kind: 'ask',
      id: 'q1',
      tool: 'Write',
      input: { file_path: '/tmp/x' },
    });
    expect(ask.text).toBe('/tmp/x');
  });

  it('records the verdict against the question it answers', () => {
    // Two questions can be open at once, and the answer names which one — the
    // reason `answered` carries an id rather than being positional.
    const seen = transcript(
      { kind: 'ask', id: 'q1', tool: 'Bash', input: { command: 'ls' } },
      { kind: 'ask', id: 'q2', tool: 'Write', input: { file_path: '/tmp/y' } },
      { kind: 'answered', id: 'q2', allowed: false },
    );
    const asks = seen.filter((e) => e.kind === 'ask');
    expect(asks.map((a) => [a.ask, a.allowed])).toEqual([
      ['q1', undefined],
      ['q2', false],
    ]);
  });
});

/** When things happened, and what the tools said — the two facts the wire used
 *  not to carry, and whose absence read as a finished feature rather than a gap. */
describe('transcript · time and detail', () => {
  /** A moment on a given day, in the reader's own timezone — which is the one the
   *  fold compares against, so the fixture has to be built in it too. */
  function on(day: number, hour: number): number {
    return new Date(2026, 7, day, hour, 30).getTime();
  }

  it('gives a tool call what its result said, not just whether it worked', () => {
    const seen = transcript(
      { kind: 'tool', id: 'a', name: 'Bash', input: { command: 'grep -c foo' } },
      { kind: 'tool_result', id: 'a', ok: true, detail: '3' },
    );
    const [tool] = seen.filter((e) => e.kind === 'tool');
    expect(tool.ok).toBe(true);
    expect(tool.detail).toBe('3');
    expect(tool.cut).toBeUndefined();
  });

  it('keeps the first line apart, because that is what the row shows', () => {
    // A result's answer is usually its first line, and one line on the row costs
    // no height at all — which on a phone is the difference between a fact being
    // there and being a tap away.
    const seen = transcript(
      { kind: 'tool', id: 'a', name: 'Bash', input: { command: 'lake build' } },
      { kind: 'tool_result', id: 'a', ok: false, detail: 'error: unknown flag\nnote: try --help' },
    );
    const [tool] = seen.filter((e) => e.kind === 'tool');
    expect(tool.head).toBe('error: unknown flag');
    expect(tool.detail).toBe('error: unknown flag\nnote: try --help');
  });

  it('says nothing extra when the whole result is one line', () => {
    // The head and the detail being equal is how the view knows there is no
    // "the rest" to offer.
    const seen = transcript(
      { kind: 'tool', id: 'a', name: 'Bash', input: { command: 'true' } },
      { kind: 'tool_result', id: 'a', ok: true, detail: 'done' },
    );
    const [tool] = seen.filter((e) => e.kind === 'tool');
    expect(tool.head).toBe(tool.detail);
  });

  it('keeps the true length when the runner cut the result', () => {
    const seen = transcript(
      { kind: 'tool', id: 'a', name: 'Read', input: { file_path: '/tmp/big' } },
      { kind: 'tool_result', id: 'a', ok: true, detail: 'x'.repeat(2000), cut: 9000 },
    );
    const [tool] = seen.filter((e) => e.kind === 'tool');
    expect(tool.cut).toBe(9000);
  });

  it('files a block under the time it began, not the time it finished', () => {
    // An answer streams for as long as it takes. Stamped at its last delta it
    // would sort after tool calls it actually preceded.
    const seen = transcript(
      { kind: 'text', text: 'I have ', at: on(3, 10) },
      { kind: 'text', text: 'read it', at: on(3, 11) },
    );
    const [said] = seen.filter((e) => e.kind === 'said');
    expect(said.at).toBe(on(3, 10));
  });

  it('puts a date in when the conversation crosses midnight', () => {
    const seen = transcript(
      { kind: 'prompt', text: 'first', at: on(2, 23) },
      { kind: 'prompt', text: 'second', at: on(3, 9) },
      { kind: 'prompt', text: 'third', at: on(3, 10) },
    );
    // One at the top for the day it opens on, one where the day changes — and
    // none between two messages on the same day.
    expect(seen.map((e) => e.kind)).toEqual(['day', 'asked', 'day', 'asked', 'asked']);
  });

  it('invents no date for a transcript that does not say when', () => {
    // A line is entitled not to carry a timestamp, and a made-up one is worse
    // than none: it would date a conversation from June today.
    const seen = transcript({ kind: 'prompt', text: 'when was this' });
    expect(seen.map((e) => e.kind)).toEqual(['asked']);
    expect(seen[0].at).toBeUndefined();
  });
});

describe('compaction', () => {
  it('marks where the session stopped remembering', () => {
    // The messages above a compaction are still on screen but are no longer in
    // the session's head, and it is where the header's exchange count starts
    // again. Without a line saying so, the page claims a memory the session
    // does not have.
    const seen = transcript(
      { kind: 'prompt', text: 'first' },
      { kind: 'compacted' },
      { kind: 'prompt', text: 'second' },
    );
    expect(seen.map((entry) => entry.kind)).toEqual(['asked', 'note', 'asked']);
    expect(seen[1].text).toContain('compacted');
  });

  it('reports a turn in requests, which is what the number counts', () => {
    // ⚠ Round trips to the model, not messages anybody sees, and not exchanges.
    // It was "turns" (which clashed with a header counting exchanges) and then
    // "replies" (which only shows as wrong on a long answer: one exchange of
    // this console's own transcript reported 54 against 83 assistant messages
    // and 53 tool calls).
    const seen = transcript({ kind: 'turn', turns: 5, duration_ms: 38401 });
    expect(seen[0].text).toContain('5 requests');
  });

  it('says a long turn in minutes and a short one in seconds', () => {
    // `1274.1s` for a twenty-one-minute turn is a number nobody reads, and its
    // last digit describes a rounding error next to the build inside it.
    expect(transcript({ kind: 'turn', turns: 54, duration_ms: 1_274_100 })[0].text).toContain(
      '21m 14s',
    );
    // And the precision is kept where it is the point: a call that either
    // returned at once or did not.
    expect(transcript({ kind: 'turn', turns: 1, duration_ms: 812 })[0].text).toContain('812ms');
    expect(transcript({ kind: 'turn', turns: 2, duration_ms: 38_401 })[0].text).toContain('38.4s');
    // An hour is where minutes stop being readable in their turn.
    expect(transcript({ kind: 'turn', turns: 9, duration_ms: 7_530_000 })[0].text).toContain(
      '2h 5m',
    );
  });
});

describe('a call that arrives twice', () => {
  const call = {
    kind: 'tool' as const,
    id: 'toolu_dup',
    name: 'Bash',
    input: { command: 'ls' },
  };

  it('is shown once, not twice', () => {
    // ⚠ **Measured on this console, not imagined.** An upgrade re-seeds from the
    // transcript while the bytes still in the child's pipe are drained by the
    // new image, and a line can be in both.
    const entries = transcript(call, call);
    expect(entries.filter((e) => e.kind === 'tool')).toHaveLength(1);
  });

  it('does not leave a row running for ever', () => {
    // The reason the duplicate matters. Only one result arrives, so with two
    // rows one of them keeps "running" — which is what a genuinely blocked
    // session looks like.
    const [tool] = transcript(call, call, {
      kind: 'tool_result',
      id: 'toolu_dup',
      ok: true,
      detail: 'done',
    });
    expect(tool.ok).toBe(true);
  });

  it('gives a result to the call it belongs to, not the newest one', () => {
    // Independent of duplicates: the runner reports background tasks that finish
    // long after the calls made since, and matching by recency gave one of them
    // the other's verdict.
    const entries = transcript(
      { kind: 'tool', id: 'toolu_slow', name: 'Bash', input: { command: 'sleep 60' } },
      { kind: 'tool', id: 'toolu_fast', name: 'Read', input: { file_path: '/tmp/x' } },
      { kind: 'tool_result', id: 'toolu_slow', ok: false, detail: 'timed out' },
    );
    const [slow, fast] = entries.filter((e) => e.kind === 'tool');
    expect(slow.ok, 'the slow call took its own verdict').toBe(false);
    expect(fast.ok, 'the newest call was given a verdict it never earned').toBeUndefined();
  });
});

describe('folding runs of tool calls', () => {
  const tool = (call: string, ok?: boolean): Entry => ({
    kind: 'tool',
    call,
    tool: 'Bash',
    text: 'git status',
    ok,
  });
  const said = (text: string): Entry => ({ kind: 'said', text });

  it('gathers consecutive calls into one block', () => {
    const found = blocks([said('before'), tool('a'), tool('b'), tool('c'), said('after')]);
    expect(found.map((b) => b.kind)).toEqual(['one', 'tools', 'one']);
    const run = found[1];
    expect(run.kind === 'tools' && run.entries.length).toBe(3);
    // Keyed by the first call, so what a reader opened stays open as the
    // transcript grows underneath it.
    expect(run.kind === 'tools' && run.key).toBe('a');
  });

  it('leaves a lone call alone', () => {
    // A group of one costs a tap and saves nothing.
    const found = blocks([said('x'), tool('a'), said('y')]);
    expect(found.map((b) => b.kind)).toEqual(['one', 'one', 'one']);
  });

  it('is broken by anything that is not a tool call', () => {
    // Two calls either side of a question were two pieces of work, and a run
    // that spanned it would say they were one.
    const found = blocks([
      tool('a'),
      tool('b'),
      { kind: 'ask', text: 'may I?' },
      tool('c'),
      tool('d'),
    ]);
    expect(found.map((b) => b.kind)).toEqual(['tools', 'one', 'tools']);
  });

  it('counts what a folded run should say about itself', () => {
    const found = ran([tool('a', true), tool('b', false), tool('c')]);
    expect(found).toEqual({ calls: 3, failed: 1, running: 1 });
  });
});

describe('transcript · a message the session has not read yet', () => {
  it('shows it at once, marked, rather than after the CLI gets to it', () => {
    // ⚠ **The wait is minutes, and it used to be invisible.** The runner writes
    // to stdin immediately; the CLI parks input that arrives mid-turn and reads
    // it in batches — twelve minutes for the oldest of four, measured from the
    // phone on 2026-08-07. With only the echo to go on there was nothing on
    // screen in the meantime, which reads exactly like a message that failed.
    const seen = transcript({ kind: 'accepted', text: 'is the gate green?' });
    expect(seen).toEqual([{ kind: 'asked', text: 'is the gate green?', queued: true }]);
  });

  it('promotes the waiting message rather than showing it twice', () => {
    // The two events are one message: the runner taking it, the CLI reading it.
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?' },
      { kind: 'prompt', text: 'is the gate green?' },
    );
    expect(seen).toEqual([{ kind: 'asked', text: 'is the gate green?', queued: undefined }]);
  });

  it('answers the oldest copy when the same words were sent twice', () => {
    // ⚠ **The shape that actually happened.** Believing the first had failed,
    // the same sentence was sent again fifteen seconds later — twice, that
    // evening. Matching the newest copy would clear the second and leave the
    // first marked as waiting for ever, which is the worse of the two lies: it
    // says a message that has been read is still stuck.
    const seen = transcript(
      { kind: 'accepted', text: 'why is it idle?' },
      { kind: 'accepted', text: 'why is it idle?' },
      { kind: 'prompt', text: 'why is it idle?' },
    );
    // The answered one is LAST, because being read moves it to where it entered
    // the conversation — see #117 and the suite below. The one still waiting
    // stays where it was sent.
    expect(seen.map((entry) => entry.queued)).toEqual([true, undefined]);
  });

  it('shows a replayed message plainly, having never seen it wait', () => {
    // A re-seed replays the transcript, which holds the conversation and not the
    // runner's own bookkeeping — so the echo arrives with nothing to promote and
    // must still produce the message.
    const seen = transcript({ kind: 'prompt', text: 'from the transcript' });
    expect(seen).toEqual([{ kind: 'asked', text: 'from the transcript' }]);
  });

  it('never marks a slash command as waiting, because nothing will answer', () => {
    // ⚠ **Measured, and it is the whole of #120.** `--replay-user-messages` does
    // not replay a command: `/context` on stdin produced `system`, a synthetic
    // `assistant` and `result`, and no user message at all. So a *waiting to be
    // read* marker on one can never be cleared — `life` wore one right through
    // the compaction it had already started.
    //
    // The runner decides which of the two events to send, so this side has no
    // rule to get wrong.
    const seen = transcript({ kind: 'command', text: '/compact' });
    expect(seen).toEqual([{ kind: 'asked', text: '/compact' }]);
  });

  it('says on the page which message went unread, and for how long', () => {
    // In the transcript and not only in the header, because it happened at a
    // moment: it belongs after the message it names, where scrolling back shows
    // what was asked and never taken.
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?' },
      { kind: 'deaf', unread: 1, seconds: 1284 },
    );
    expect(seen.map((entry) => entry.kind)).toEqual(['asked', 'note']);
    expect(seen[1].text).toBe('not reading — 1 message written and untouched for 21m 24s');
  });

  it('reads a command back out of the transcript, where it used to vanish', () => {
    // The CLI writes a command down as a `<command-name>` wrapper, which the
    // reader dropped as plumbing — so scrolling back through a conversation gave
    // no sign that anyone had ever compacted it. Sent and recorded arrive as the
    // same event by design: it is one thing that happened.
    const seen = transcript(
      { kind: 'command', text: '/compact' },
      { kind: 'command', text: '/loop check eval output' },
    );
    expect(seen.map((entry) => entry.text)).toEqual(['/compact', '/loop check eval output']);
  });
});

describe('transcript · an answer the session has not acted on', () => {
  const ASK = { kind: 'ask' as const, id: 'q1', tool: 'AskUserQuestion', title: 'which way?' };

  it('does not claim the session has it merely because it was written', () => {
    // ⚠ **The defect this replaces was affirmatively wrong, in green.**
    // `Answered` is pushed once the decision reaches the pipe, and the card drew
    // its verdict from that — so `health` showed *answered* for thirty-one
    // minutes while blocked on the very same question (memview #122).
    const seen = transcript(ASK, { kind: 'answered', id: 'q1', allowed: true });
    expect(seen[0].allowed).toBe(true);
    expect(seen[0].settling, 'written, not yet taken up').toBe(true);
  });

  it('takes the session speaking as the receipt', () => {
    // There is no dedicated echo for a decision the way a prompt has its replay.
    // None is needed: the question blocked the turn, so anything at all
    // afterwards means the answer was read.
    const seen = transcript(
      ASK,
      { kind: 'answered', id: 'q1', allowed: true },
      { kind: 'tool', id: 't1', name: 'Bash', input: { command: 'ls' } },
    );
    expect(seen[0].settling).toBeUndefined();
  });

  it('does not take a status announcement as one', () => {
    // ⚠ A status is announced only when it CHANGES (memview #112), so `busy` can
    // arrive from a session that then goes silent for half an hour — which is
    // the state this is here to tell apart.
    const seen = transcript(
      ASK,
      { kind: 'answered', id: 'q1', allowed: true },
      { kind: 'busy', status: 'requesting' },
    );
    expect(seen[0].settling).toBe(true);
  });
});

describe('transcript · where a message that waited belongs', () => {
  it('moves it below the work that happened before it was read', () => {
    // ⚠ **A defect the queued marker introduced.** The entry goes in when the
    // runner takes the message, and the CLI may not read it for minutes — so
    // everything the session did meanwhile was appended below it, reading as
    // though the message had been seen first (memview #117).
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?', at: 1000 },
      { kind: 'tool', id: 't1', name: 'Bash', input: { command: 'cargo test' } },
      { kind: 'text', text: 'still on the last thing' },
      { kind: 'prompt', text: 'is the gate green?', at: 9000 },
    );
    // The leading `day` is the date row any timestamped entry introduces.
    expect(seen.map((e) => e.kind)).toEqual(['day', 'tool', 'said', 'asked']);
    expect(seen[3].queued).toBeUndefined();
    expect(seen[3].at, 'stamped when it was read, so the clock stays monotonic').toBe(9000);
  });

  it('moves the oldest copy when the same words were sent twice', () => {
    // ⚠ Extends the matching rule rather than replacing it: believing the first
    // had failed, the same sentence was sent again fifteen seconds later —
    // twice, in one evening. The echo answers the oldest waiting copy, and the
    // move has to take that same entry.
    const seen = transcript(
      { kind: 'accepted', text: 'why is it idle?', at: 1000 },
      { kind: 'accepted', text: 'why is it idle?', at: 2000 },
      { kind: 'prompt', text: 'why is it idle?', at: 9000 },
    );
    expect(seen.filter((e) => e.kind === 'asked').map((e) => [e.at, e.queued])).toEqual([
      [2000, true],
      [9000, undefined],
    ]);
  });

  it('leaves a message alone while it is still waiting', () => {
    // Where it waits is the sender's own timeline, and showing it there at once
    // is the whole of what stops the re-sending.
    const seen = transcript(
      { kind: 'accepted', text: 'have a look', at: 1000 },
      { kind: 'tool', id: 't1', name: 'Bash', input: { command: 'ls' } },
    );
    expect(seen.map((e) => e.kind)).toEqual(['day', 'asked', 'tool']);
    expect(seen[1].queued).toBe(true);
  });
});
