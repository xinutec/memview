import { describe, expect, it } from 'vitest';

import { gather, ranged, segments, type Gathered } from './loops';
import type { Line, Pass } from './models';

function line(argv: string, passes: Pass[] = [], writes: string[] = []): Line {
  return {
    depth: 0,
    argv: argv.split(' '),
    reached: 'always',
    scope: [],
    kind: 'run',
    key: 'run',
    says: '',
    uses: writes.map((path) => ({ path, write: true, reached: 'always', certain: true })),
    passes,
  };
}

const pass = (name: string, value: string, at: number): Pass => ({ name, value, at });

/** A string with its slots as `{name}`, for reading in an assertion. */
const shown = (text: string) =>
  segments(text)
    .map((s) => (s.slot ? `{${s.text}}` : s.text))
    .join('');

/** The shape alone: a step's words, or a loop's head, values, shape and misfits. */
function shape(nodes: readonly Gathered[]): unknown[] {
  return nodes.map((node) =>
    node.kind === 'step'
      ? shown(node.step.argv.join(' '))
      : {
          head: node.head.argv.join(' '),
          values: node.values,
          shape: shape(node.shape),
          ...(node.misfits.length && {
            misfits: node.misfits.map((m) => [m.value, shape(m.nodes)]),
          }),
        },
  );
}

/** `for name in values; do <body with $name>; done`, as the reader runs it out. */
function looped(name: string, values: string[], body: (value: string) => string[]): Line[] {
  return [
    line(`for ${name} in ${values.join(' ')}`),
    ...values.flatMap((value, at) =>
      body(value).map((argv) => line(argv, [pass(name, value, at)])),
    ),
  ];
}

describe('gather', () => {
  it('leaves a script without loops as it is', () => {
    expect(shape(gather([line('ls'), line('pwd')]))).toEqual(['ls', 'pwd']);
  });

  it('draws the body once, the value as a slot', () => {
    const steps = [...looped('f', ['a', 'b'], (v) => [`wc ${v}.txt`, `rm ${v}.txt`]), line('ls')];
    expect(shape(gather(steps))).toEqual([
      { head: 'for f in a b', values: ['a', 'b'], shape: ['wc {f}.txt', 'rm {f}.txt'] },
      'ls',
    ]);
  });

  it('puts the slot in the paths a pass writes', () => {
    const steps = [
      line('for r in x y'),
      line('git status', [pass('r', 'x', 0)], ['/tmp/s-x']),
      line('git status', [pass('r', 'y', 1)], ['/tmp/s-y']),
    ];
    const [loop] = gather(steps);
    if (loop?.kind !== 'loop') throw new Error('not a loop');
    const [step] = loop.shape;
    if (step?.kind !== 'step') throw new Error('not a step');
    expect(step.step.uses.map((used) => shown(used.path))).toEqual(['/tmp/s-{r}']);
  });

  it('keeps text that only looks like the value', () => {
    // `-1` is the same in every pass; the `1` in the path is the value.
    const steps = looped('i', ['1', '2', '3'], (v) => [`head -1 /tmp/s${v}`]);
    expect(shape(gather(steps))).toEqual([
      { head: 'for i in 1 2 3', values: ['1', '2', '3'], shape: ['head -1 /tmp/s{i}'] },
    ]);
  });

  it('draws a pass that does not fit the shape as it ran', () => {
    const steps = looped('h', ['a', 'b', 'c'], (v) =>
      v === 'b' ? ['ssh b', 'reboot'] : [`ping ${v}`],
    );
    expect(shape(gather(steps))).toEqual([
      {
        head: 'for h in a b c',
        values: ['a', 'c'],
        shape: ['ping {h}'],
        misfits: [['b', ['ssh b', 'reboot']]],
      },
    ]);
  });

  it('takes its shape from the most common pass, not the first', () => {
    const steps = looped('n', ['0', '1', '2'], (v) => (v === '0' ? ['skip'] : [`run ${v}`]));
    expect(shape(gather(steps))).toEqual([
      {
        head: 'for n in 0 1 2',
        values: ['1', '2'],
        shape: ['run {n}'],
        misfits: [['0', ['skip']]],
      },
    ]);
  });

  it('keeps two loops in a row apart', () => {
    const steps = [
      ...looped('f', ['a'], (v) => [`cat ${v}`]),
      ...looped('g', ['b'], (v) => [`cat ${v}`]),
    ];
    expect(shape(gather(steps))).toEqual([
      { head: 'for f in a', values: ['a'], shape: ['cat a'] },
      { head: 'for g in b', values: ['b'], shape: ['cat b'] },
    ]);
  });

  it('nests a loop inside a loop, each with its own slot', () => {
    const steps = [line('for d in x y')];
    for (const [i, d] of ['x', 'y'].entries()) {
      const outer = pass('d', d, i);
      steps.push(line('for f in a b', [outer]));
      for (const [j, f] of ['a', 'b'].entries())
        steps.push(line(`cat ${d}/${f}`, [outer, pass('f', f, j)]));
    }
    expect(shape(gather(steps))).toEqual([
      {
        head: 'for d in x y',
        values: ['x', 'y'],
        shape: [{ head: 'for f in a b', values: ['a', 'b'], shape: ['cat {d}/{f}'] }],
      },
    ]);
  });

  it("folds the last pass's surer `&&` into the shape, claiming the weaker", () => {
    // Only a loop's last pass reports its status, so only its `&&` is confirmable.
    // The `ls` of pass 1 (index 2) is unconfirmable; pass 2's (the last) is not.
    const steps = looped('d', ['a', 'b'], (v) => [`cd ${v}`, 'ls']).map((step, at) =>
      at === 2
        ? { ...step, reached: 'sometimes' as const }
        : at === 4
          ? { ...step, reached: 'on-success' as const }
          : step,
    );
    const [loop] = gather(steps);
    if (loop?.kind !== 'loop') throw new Error('not a loop');
    expect(loop.misfits).toEqual([]);
    expect(loop.shape.map((node) => node.kind === 'step' && node.step.reached)).toEqual([
      'always',
      'sometimes',
    ]);
  });

  it('tells two passes over the same value apart', () => {
    const [loop] = gather(looped('f', ['a', 'a'], (v) => [`cat ${v}`]));
    expect(loop?.kind === 'loop' && [loop.passes, loop.values]).toEqual([2, ['a', 'a']]);
  });
});

describe('ranged', () => {
  it('folds a run of integers', () => {
    expect(ranged(Array.from({ length: 120 }, (_, n) => String(n + 1)))).toBe('1–120');
  });

  it('leaves short runs and words alone', () => {
    expect(ranged(['1', '2', 'x', '4', '5', '6', '9'])).toBe('1, 2, x, 4–6, 9');
  });
});
