import { describe, expect, it } from 'vitest';

import {
  type Call,
  type Entry,
  type Questioned,
  type Timed,
  type ToolCall,
  asking,
} from './models';
import { blocks, fold, ran } from './transcript';
import { first, nth } from './testing';

function transcript(...events: Timed[]): Entry[] {
  return events.reduce<Entry[]>((entries, event) => fold(entries, event), []);
}

/** The entry, asserting what kind it is; the test then reads it as that. */
function only<K extends Entry['kind']>(entry: Entry | undefined, kind: K): Entry & { kind: K } {
  if (entry?.kind !== kind) throw new Error(`expected a ${kind}, got ${entry?.kind}`);
  return entry as Entry & { kind: K };
}

function asked(entry: Entry | undefined): Questioned {
  if (!entry || !asking(entry)) throw new Error(`expected a question, got ${entry?.kind}`);
  return entry;
}

const tools = (entries: readonly Entry[]): ToolCall[] =>
  entries.filter((entry): entry is ToolCall => entry.kind === 'tool');

const tool = (id: string, name: string, does: Call): Timed => ({
  kind: 'tool',
  id,
  name,
  does,
});
/** Calls as the runner reads them; see `console/src/call.rs`. */
const bash = (command: string): Call => ({ kind: 'bash', command });
const other = (shown: string): Call => ({ kind: 'other', shown });
const edit = (path: string, before: string, after: string, everywhere = false): Call => ({
  kind: 'edit',
  path,
  before,
  after,
  everywhere,
});
const result = (id: string, ok: boolean, detail = '', cut?: number): Timed => ({
  kind: 'tool_result',
  id,
  ok,
  detail,
  ...(cut === undefined ? {} : { cut }),
});
const ask = (
  id: string,
  tool: string,
  does: Call,
  more?: Partial<Timed & { kind: 'ask' }>,
): Timed => ({
  kind: 'ask',
  id,
  tool,
  does,
  ...more,
});
const turn = (turns: number, duration_ms: number): Timed => ({
  kind: 'turn',
  cost_usd: 0,
  turns,
  duration_ms,
});
const joined = (earlier: number, restarted = false): Timed => ({
  kind: 'joined',
  earlier,
  from: 0,
  restarted,
});

describe('transcript', () => {
  it('joins the deltas of one answer into one paragraph', () => {
    const seen = transcript(
      { kind: 'text', text: 'I have ' },
      { kind: 'text', text: 'read the file' },
      { kind: 'text', text: ' and it is fine.' },
    );
    expect(seen).toEqual([{ kind: 'said', text: 'I have read the file and it is fine.' }]);
  });

  it('gives a tool result to the call it answers, not to the newest line', () => {
    const seen = transcript(
      tool('a', 'Read', other('/tmp/one')),
      tool('b', 'Bash', bash('ls')),
      result('b', false),
      result('a', true),
    );
    expect(tools(seen).map((t) => [t.tool, t.ok])).toEqual([
      ['Read', true],
      ['Bash', false],
    ]);
  });

  it('shows a call with no result yet as running', () => {
    const call = first(tools(transcript(tool('a', 'Bash', bash('sleep 60')))));
    expect(call.ok).toBeUndefined();
    expect(call.text).toBe('sleep 60');
  });

  it('names a tool call by its most telling argument', () => {
    const read = first(tools(transcript(tool('a', 'Read', other('/etc/hosts')))));
    expect(read.text).toBe('/etc/hosts');
    const task = first(tools(transcript(tool('b', 'Task', other('subagent_type')))));
    expect(task.text).toBe('subagent_type');
  });

  it('leaves session state out of the transcript', () => {
    expect(
      transcript(
        { kind: 'started', model: 'x', cwd: '/tmp', tools: 3 },
        { kind: 'busy', status: 'requesting' },
        { kind: 'limit', window: 'five_hour', status: 'allowed' },
      ),
    ).toEqual([]);
  });

  it('says when a session ended, and how', () => {
    expect(only(transcript({ kind: 'exited', code: 0 })[0], 'note').text).toBe('session ended');
    expect(only(transcript({ kind: 'exited' })[0], 'note').text).toContain('killed');
  });
});

describe('questions', () => {
  it('shows a question as undecided until it is answered', () => {
    const question = first(
      transcript(
        ask('q1', 'Bash', bash('rm -rf build'), { title: 'Claude wants to run rm -rf build' }),
      ),
    );
    expect(question.kind).toBe('ask');
    expect(asked(question).ask).toBe('q1');
    expect(asked(question).allowed).toBeUndefined();
    expect(asked(question).text).toBe('Claude wants to run rm -rf build');
  });

  it('carries the options of a question through to the entry', () => {
    const question = first(
      transcript(
        ask('q1', 'AskUserQuestion', {
          kind: 'question',
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
        }),
      ),
    );
    expect(first(asked(question).questions ?? []).options.map((o) => o.label)).toEqual([
      'left',
      'right',
    ]);
  });

  it('leaves every other tool without options, which is what keeps allow and refuse', () => {
    const question = first(transcript(ask('q1', 'Bash', other('questions'))));
    expect(asked(question).questions).toBeUndefined();
  });

  it('falls back to the arguments when the CLI offers no sentence', () => {
    const question = first(transcript(ask('q1', 'Write', other('/tmp/x'))));
    expect(asked(question).text).toBe('/tmp/x');
  });

  it('asks on the call it is about rather than beside it', () => {
    const seen = transcript(
      tool('toolu_1', 'Write', other('/tmp/x')),
      ask('q1', 'Write', other('/tmp/x'), { call: 'toolu_1' }),
    );
    expect(seen.length).toBe(1);
    const call = only(seen[0], 'tool');
    expect(call.ask).toBe('q1');
    expect(call.allowed).toBeUndefined();
  });

  it('leaves the answered call as an ordinary tool row', () => {
    const seen = transcript(
      tool('toolu_1', 'Write', other('/tmp/x')),
      ask('q1', 'Write', other('/tmp/x'), { call: 'toolu_1' }),
      { kind: 'answered', id: 'q1', allowed: true },
      result('toolu_1', true, 'written'),
    );
    expect(seen.length).toBe(1);
    const call = only(seen[0], 'tool');
    expect(call.allowed).toBe(true);
    expect(call.ok).toBe(true);
    expect(call.head).toBe('written');
  });

  it('still draws a question that names no call', () => {
    const seen = transcript(ask('q1', 'WebFetch', other('host')));
    expect(seen.map((e) => e.kind)).toEqual(['ask']);
    expect(asked(seen[0]).ask).toBe('q1');
  });

  it('does not attach a question to a call it does not name', () => {
    const seen = transcript(
      tool('toolu_1', 'Write', other('/tmp/x')),
      ask('q1', 'Write', other('/tmp/x'), { call: 'toolu_other' }),
    );
    expect(seen.map((e) => e.kind)).toEqual(['tool', 'ask']);
  });

  it('records the verdict against the question it answers', () => {
    const seen = transcript(ask('q1', 'Bash', bash('ls')), ask('q2', 'Write', other('/tmp/y')), {
      kind: 'answered',
      id: 'q2',
      allowed: false,
    });
    expect(seen.map((e) => [asked(e).ask, asked(e).allowed])).toEqual([
      ['q1', undefined],
      ['q2', false],
    ]);
  });
});

describe('transcript · time and detail', () => {
  function on(day: number, hour: number): number {
    return new Date(2026, 7, day, hour, 30).getTime();
  }

  it('gives a tool call what its result said, not just whether it worked', () => {
    const call = first(
      tools(transcript(tool('a', 'Bash', bash('grep -c foo')), result('a', true, '3'))),
    );
    expect(call.ok).toBe(true);
    expect(call.detail).toBe('3');
    expect(call.cut).toBeUndefined();
  });

  it('keeps the first line apart, because that is what the row shows', () => {
    const call = first(
      tools(
        transcript(
          tool('a', 'Bash', bash('lake build')),
          result('a', false, 'error: unknown flag\nnote: try --help'),
        ),
      ),
    );
    expect(call.head).toBe('error: unknown flag');
    expect(call.detail).toBe('error: unknown flag\nnote: try --help');
  });

  it('says nothing extra when the whole result is one line', () => {
    const call = first(
      tools(transcript(tool('a', 'Bash', bash('true')), result('a', true, 'done'))),
    );
    expect(call.head).toBe(call.detail);
  });

  it('keeps the true length when the runner cut the result', () => {
    const call = first(
      tools(
        transcript(tool('a', 'Read', other('/tmp/big')), result('a', true, 'x'.repeat(2000), 9000)),
      ),
    );
    expect(call.cut).toBe(9000);
  });

  it('files a block under the time it began, not the time it finished', () => {
    const seen = transcript(
      { kind: 'text', text: 'I have ', at: on(3, 10) },
      { kind: 'text', text: 'read it', at: on(3, 11) },
    );
    expect(
      only(
        seen.find((e) => e.kind === 'said'),
        'said',
      ).at,
    ).toBe(on(3, 10));
  });

  it('puts a date in when the conversation crosses midnight', () => {
    const seen = transcript(
      { kind: 'prompt', text: 'first', at: on(2, 23) },
      { kind: 'prompt', text: 'second', at: on(3, 9) },
      { kind: 'prompt', text: 'third', at: on(3, 10) },
    );
    expect(seen.map((e) => e.kind)).toEqual(['day', 'asked', 'day', 'asked', 'asked']);
  });

  it('invents no date for a transcript that does not say when', () => {
    const seen = transcript({ kind: 'prompt', text: 'when was this' });
    expect(seen.map((e) => e.kind)).toEqual(['asked']);
    expect(first(seen).at).toBeUndefined();
  });
});

describe('compaction', () => {
  it('marks where the session stopped remembering', () => {
    const seen = transcript(
      { kind: 'prompt', text: 'first' },
      { kind: 'compacted' },
      { kind: 'prompt', text: 'second' },
    );
    expect(seen.map((entry) => entry.kind)).toEqual(['asked', 'note', 'asked']);
    expect(only(seen[1], 'note').text).toContain('compacted');
  });

  it('reports a turn in requests, which is what the number counts', () => {
    expect(only(transcript(turn(5, 38401))[0], 'turn').text).toContain('5 requests');
  });

  it('says a long turn in minutes and a short one in seconds', () => {
    const said = (turns: number, ms: number) => only(transcript(turn(turns, ms))[0], 'turn').text;
    expect(said(54, 1_274_100)).toContain('21m 14s');
    expect(said(1, 812)).toContain('812ms');
    expect(said(2, 38_401)).toContain('38.4s');
    expect(said(9, 7_530_000)).toContain('2h 5m');
  });
});

describe('a call that arrives twice', () => {
  const call = tool('toolu_dup', 'Bash', bash('ls'));

  it('is shown once, not twice', () => {
    expect(tools(transcript(call, call))).toHaveLength(1);
  });

  it('does not leave a row running for ever', () => {
    const seen = first(tools(transcript(call, call, result('toolu_dup', true, 'done'))));
    expect(seen.ok).toBe(true);
  });

  it('gives a result to the call it belongs to, not the newest one', () => {
    const both = tools(
      transcript(
        tool('toolu_slow', 'Bash', bash('sleep 60')),
        tool('toolu_fast', 'Read', other('/tmp/x')),
        result('toolu_slow', false, 'timed out'),
      ),
    );
    const slow = first(both);
    const fast = nth(both, 1);
    expect(slow.ok, 'the slow call took its own verdict').toBe(false);
    expect(fast.ok, 'the newest call was given a verdict it never earned').toBeUndefined();
  });
});

describe('folding runs of tool calls', () => {
  const call = (id: string, ok?: boolean): ToolCall => ({
    kind: 'tool',
    call: id,
    tool: 'Bash',
    text: 'git status',
    ok,
  });
  const said = (text: string): Entry => ({ kind: 'said', text });

  it('gathers consecutive calls into one block', () => {
    const found = blocks([said('before'), call('a'), call('b'), call('c'), said('after')]);
    expect(found.map((b) => b.kind)).toEqual(['one', 'tools', 'one']);
    const run = nth(found, 1);
    expect(run.kind === 'tools' && run.entries.length).toBe(3);
    expect(run.kind === 'tools' && run.key).toBe('a');
  });

  it('leaves a lone call alone', () => {
    const found = blocks([said('x'), call('a'), said('y')]);
    expect(found.map((b) => b.kind)).toEqual(['one', 'one', 'one']);
  });

  it('is broken by anything that is not a tool call', () => {
    const found = blocks([
      call('a'),
      call('b'),
      { kind: 'ask', ask: 'q', tool: 'Bash', text: 'may I?' },
      call('c'),
      call('d'),
    ]);
    expect(found.map((b) => b.kind)).toEqual(['tools', 'one', 'tools']);
  });

  it('never folds away a call that is waiting to be allowed', () => {
    const waiting: Entry = { ...call('c'), ask: 'q1' };
    const found = blocks([call('a'), call('b'), waiting, call('d'), call('e')]);
    expect(found.map((b) => b.kind)).toEqual(['tools', 'one', 'tools']);
  });

  it('folds it back in the moment it is answered', () => {
    const answered: Entry = { ...call('c'), ask: 'q1', allowed: true };
    const found = blocks([call('a'), call('b'), answered, call('d')]);
    expect(found.map((b) => b.kind)).toEqual(['tools']);
    const run = first(found);
    expect(run.kind === 'tools' && run.entries.length).toBe(4);
    expect(run.kind === 'tools' && run.key).toBe('a');
  });

  it('counts what a folded run should say about itself', () => {
    const found = ran([call('a', true), call('b', false), call('c')]);
    expect(found).toEqual({ calls: 3, failed: 1, running: 1, unrecorded: 0, images: 0 });
  });

  it('counts the pictures in a folded run', () => {
    const shot: ToolCall = { ...call('a', true), picture: '/tmp/shot.png' };
    expect(ran([shot, call('b', true)]).images).toBe(1);
  });
});

describe('transcript · a message the session has not read yet', () => {
  it('shows it at once, marked, rather than after the CLI gets to it', () => {
    const seen = transcript({ kind: 'accepted', text: 'is the gate green?' });
    expect(seen).toEqual([{ kind: 'asked', text: 'is the gate green?', queued: true }]);
  });

  it('promotes the waiting message rather than showing it twice', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?' },
      { kind: 'prompt', text: 'is the gate green?' },
    );
    expect(seen).toEqual([{ kind: 'asked', text: 'is the gate green?', queued: undefined }]);
  });

  it('answers the oldest copy when the same words were sent twice', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'why is it idle?' },
      { kind: 'accepted', text: 'why is it idle?' },
      { kind: 'prompt', text: 'why is it idle?' },
    );
    expect(seen.map((entry) => only(entry, 'asked').queued)).toEqual([true, undefined]);
  });

  it('does not break the model mid-sentence when a message is sent while it types', () => {
    // The defect: one message became TWO. A queued message is added the
    // moment it is sent, so the next delta found an `asked` at the end and
    // started a fresh `said`. On screen that is a paragraph break in the middle
    // of a sentence — and if the split falls inside a list or a code fence, BOTH
    // halves render as broken markdown.
    //
    // It did not interrupt anything: the CLI parks a message sent mid-turn and
    // reads it when the turn ends. So the model's words stay one block and the
    // message sits after them, which is also the order it was read in.
    const seen = transcript(
      { kind: 'text', text: '- one\n- two\n' },
      { kind: 'accepted', text: 'sent while it was typing' },
      { kind: 'text', text: '- three\n' },
    );
    expect(seen.map((entry) => entry.kind)).toEqual(['said', 'asked']);
    expect(only(first(seen), 'said').text).toBe('- one\n- two\n- three\n');
  });

  it('still starts a new block after a tool call, which DID interrupt', () => {
    // The control. Text either side of a tool call is two blocks and must stay
    // two — a fix that skipped back past anything would merge these.
    const seen = transcript(
      { kind: 'text', text: 'before' },
      { kind: 'tool', id: 'toolu_1', name: 'Bash', does: bash('ls') },
      { kind: 'text', text: 'after' },
    );
    expect(seen.filter((entry) => entry.kind === 'said')).toHaveLength(2);
  });

  it('carries on through several messages sent in a row', () => {
    const seen = transcript(
      { kind: 'text', text: 'one ' },
      { kind: 'accepted', text: 'first' },
      { kind: 'accepted', text: 'second' },
      { kind: 'text', text: 'two' },
    );
    expect(only(first(seen), 'said').text).toBe('one two');
    expect(seen.map((entry) => entry.kind)).toEqual(['said', 'asked', 'asked']);
  });

  it('shows a replayed message plainly, having never seen it wait', () => {
    const seen = transcript({ kind: 'prompt', text: 'from the transcript' });
    expect(seen).toEqual([{ kind: 'asked', text: 'from the transcript' }]);
  });

  it('never marks a slash command as waiting, because nothing will answer', () => {
    const seen = transcript({ kind: 'command', text: '/compact' });
    expect(seen).toEqual([{ kind: 'asked', text: '/compact' }]);
  });

  it('says on the page which message went unread, and for how long', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?' },
      { kind: 'deaf', unread: 1, seconds: 1284 },
    );
    expect(seen.map((entry) => entry.kind)).toEqual(['asked', 'note']);
    expect(only(seen[1], 'note').text).toBe(
      'not reading — 1 message written and untouched for 21m 24s',
    );
  });

  it('reads a command back out of the transcript, where it used to vanish', () => {
    const seen = transcript(
      { kind: 'command', text: '/compact' },
      { kind: 'command', text: '/loop check eval output' },
    );
    expect(seen.map((entry) => only(entry, 'asked').text)).toEqual([
      '/compact',
      '/loop check eval output',
    ]);
  });
});

describe('transcript · an answer the session has not acted on', () => {
  const ASK = ask('q1', 'AskUserQuestion', other('AskUserQuestion'), { title: 'which way?' });

  it('does not claim the session has it merely because it was written', () => {
    const seen = transcript(ASK, { kind: 'answered', id: 'q1', allowed: true });
    expect(asked(seen[0]).allowed).toBe(true);
    expect(asked(seen[0]).settling, 'written, not yet taken up').toBe(true);
  });

  it('takes the session speaking as the receipt', () => {
    const seen = transcript(
      ASK,
      { kind: 'answered', id: 'q1', allowed: true },
      tool('t1', 'Bash', bash('ls')),
    );
    expect(asked(seen[0]).settling).toBeUndefined();
  });

  it('does not take a status announcement as one', () => {
    const seen = transcript(
      ASK,
      { kind: 'answered', id: 'q1', allowed: true },
      { kind: 'busy', status: 'requesting' },
    );
    expect(asked(seen[0]).settling).toBe(true);
  });
});

describe('transcript · where a message that waited belongs', () => {
  it('moves it below the work that happened before it was read', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'is the gate green?', at: 1000 },
      tool('t1', 'Bash', bash('cargo test')),
      { kind: 'text', text: 'still on the last thing' },
      { kind: 'prompt', text: 'is the gate green?', at: 9000 },
    );
    expect(seen.map((e) => e.kind)).toEqual(['day', 'tool', 'said', 'asked']);
    const read = only(seen[3], 'asked');
    expect(read.queued).toBeUndefined();
    expect(read.at, 'stamped when it was read, so the clock stays monotonic').toBe(9000);
  });

  it('moves the oldest copy when the same words were sent twice', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'why is it idle?', at: 1000 },
      { kind: 'accepted', text: 'why is it idle?', at: 2000 },
      { kind: 'prompt', text: 'why is it idle?', at: 9000 },
    );
    expect(
      seen.filter((e) => e.kind === 'asked').map((e) => [e.at, only(e, 'asked').queued]),
    ).toEqual([
      [2000, true],
      [9000, undefined],
    ]);
  });

  it('leaves a message alone while it is still waiting', () => {
    const seen = transcript(
      { kind: 'accepted', text: 'have a look', at: 1000 },
      tool('t1', 'Bash', bash('ls')),
    );
    expect(seen.map((e) => e.kind)).toEqual(['day', 'asked', 'tool']);
    expect(only(seen[1], 'asked').queued).toBe(true);
  });
});

describe('a call whose result was never written', () => {
  it('stops claiming a call from before the console joined is running', () => {
    const call = first(
      tools(transcript(tool('dead', 'Bash', bash('home-manager switch')), joined(2, true))),
    );
    expect(call.unrecorded).toBe(true);
    expect(call.ok).toBeUndefined();
  });

  it('lets a call that was genuinely in flight correct itself', () => {
    const call = first(
      tools(
        transcript(
          tool('live', 'Bash', bash('sleep 60')),
          joined(1, true),
          result('live', true, 'done'),
        ),
      ),
    );
    expect(call.unrecorded).toBeUndefined();
    expect(call.ok).toBe(true);
  });

  it('does not call a running task lost when a reader merely joined', () => {
    const seen = tools(transcript(tool('live', 'Bash', bash('sleep 600')), joined(1)));
    expect(first(seen).unrecorded).toBeUndefined();
    expect(first(seen).ok).toBeUndefined();
    expect(ran(seen)).toEqual({ calls: 1, failed: 0, running: 1, unrecorded: 0, images: 0 });
  });

  it('leaves calls made after the boundary alone', () => {
    const call = first(tools(transcript(joined(0), tool('now', 'Bash', bash('ls')))));
    expect(call.unrecorded).toBeUndefined();
    expect(call.ok).toBeUndefined();
  });

  it('counts an unrecorded call as neither running nor failed', () => {
    const seen = tools(
      transcript(
        tool('dead', 'Bash', bash('x')),
        tool('ok', 'Read', other('/tmp/a')),
        joined(2, true),
        result('ok', true),
      ),
    );
    expect(ran(seen)).toEqual({ calls: 2, failed: 0, running: 0, unrecorded: 1, images: 0 });
  });

  it('counts one earlier event in the singular', () => {
    expect(transcript(joined(1))[0]).toEqual({
      kind: 'note',
      text: '1 earlier event, read from the transcript',
    });
    expect(transcript(joined(2))[0]).toEqual({
      kind: 'note',
      text: '2 earlier events, read from the transcript',
    });
  });
});

describe('a picture a call returned', () => {
  const read = tool('r1', 'Read', other('/tmp/shot.png'));
  const picture: Timed = {
    kind: 'tool_result',
    id: 'r1',
    ok: true,
    detail: '[an image]',
    image: true,
  };

  it('is shown from the path the call read', () => {
    expect(first(tools(transcript(read, picture))).picture).toBe('/tmp/shot.png');
  });

  it('is not guessed from words', () => {
    expect(first(tools(transcript(read, result('r1', true, 'a page')))).picture).toBeUndefined();
  });

  it('needs a path the console can open', () => {
    const fetched = tool('r1', 'WebFetch', other('what is on it'));
    expect(first(tools(transcript(fetched, picture))).picture).toBeUndefined();
  });
});

describe('an edit', () => {
  it('keeps what it replaced and what replaced it', () => {
    const call = tool('e1', 'Edit', edit('/tmp/a.rs', 'let x = 1;', 'let x = 2;'));
    expect(first(tools(transcript(call))).change).toEqual({
      path: '/tmp/a.rs',
      before: 'let x = 1;',
      after: 'let x = 2;',
      everywhere: false,
    });
  });

  it('says when it replaced every occurrence', () => {
    const call = tool('e1', 'Edit', edit('/tmp/a.rs', 'a', 'b', true));
    expect(first(tools(transcript(call))).change?.everywhere).toBe(true);
  });

  it('can be read before it is allowed, from the question about it', () => {
    const asking: Timed = {
      kind: 'ask',
      id: 'q1',
      tool: 'Edit',
      does: edit('/tmp/a.rs', 'a', 'b'),
    };
    const entry = asked(first(transcript(asking)));
    expect(entry.change?.after).toBe('b');
  });

  it('is nothing for a call that is not an edit', () => {
    expect(first(tools(transcript(tool('b1', 'Bash', bash('ls'))))).change).toBeUndefined();
  });
});
