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
