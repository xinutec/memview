import { describe, expect, it } from 'vitest';

import { Entry, SessionEvent } from './models';
import { fold } from './transcript';

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

  it('keeps thinking out of the answer', () => {
    const seen = transcript(
      { kind: 'thinking', text: 'let me check' },
      { kind: 'text', text: 'done' },
      { kind: 'thinking', text: 'more' },
    );
    expect(seen.map((e) => e.kind)).toEqual(['thought', 'said', 'thought']);
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
    const [tool] = transcript({ kind: 'tool', id: 'a', name: 'Bash', input: { command: 'sleep 60' } });
    expect(tool.ok).toBeUndefined();
    expect(tool.text).toBe('sleep 60');
  });

  it('names a tool call by its most telling argument', () => {
    const [read] = transcript({ kind: 'tool', name: 'Read', input: { file_path: '/etc/hosts', limit: 5 } });
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

  it('falls back to the arguments when the CLI offers no sentence', () => {
    const [ask] = transcript({ kind: 'ask', id: 'q1', tool: 'Write', input: { file_path: '/tmp/x' } });
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
