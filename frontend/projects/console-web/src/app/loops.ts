import type { Line, Pass, Reach } from './models';

/**
 * A step on the sheet, or a loop the reader ran out, drawn as its body once with
 * the loop variable as a slot.
 */
export type Gathered =
  | { readonly kind: 'step'; readonly at: number; readonly step: Line }
  | {
      readonly kind: 'loop';
      readonly at: number;
      readonly head: Line;
      readonly name: string;
      /** Every pass, fitting or not. */
      readonly passes: number;
      /** The values of the passes the shape stands for, in running order. */
      readonly values: readonly string[];
      /** The body once, its strings holding [[slot]] markers where the value went. */
      readonly shape: readonly Gathered[];
      /** Passes that do not fit the shape, drawn as they ran. */
      readonly misfits: readonly Misfit[];
    };

export interface Misfit {
  /** Which pass, from 0. */
  readonly at: number;
  readonly value: string;
  readonly nodes: readonly Gathered[];
}

/** A run of text, or the slot a loop variable's value filled. */
export interface Segment {
  readonly text: string;
  readonly slot?: boolean;
}

const OPEN = '';
const CLOSE = '';
const MARKED = /\d+:([^]*)/g;

/** The marker standing for the variable of the loop at `depth`, as written into a string. */
function slot(depth: number, name: string): string {
  return `${OPEN}${depth}:${name}${CLOSE}`;
}

/** A string from the sheet, cut into text and the slots in it. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  let from = 0;
  for (const match of text.matchAll(MARKED)) {
    if (match.index > from) out.push({ text: text.slice(from, match.index) });
    out.push({ text: match[1] ?? '', slot: true });
    from = match.index + match[0].length;
  }
  if (from < text.length) out.push({ text: text.slice(from) });
  return out;
}

/** Values as a short list: a run of three or more consecutive integers is `a–b`. */
export function ranged(values: readonly string[]): string {
  const out: string[] = [];
  let run: number[] = [];
  const flush = () => {
    if (run.length >= 3) out.push(`${run[0]}–${run.at(-1)}`);
    else out.push(...run.map(String));
    run = [];
  };
  for (const value of values) {
    const n = /^-?\d+$/.test(value) ? Number(value) : undefined;
    if (n !== undefined && (run.length === 0 || n === (run.at(-1) ?? NaN) + 1)) {
      run.push(n);
      continue;
    }
    flush();
    if (n === undefined) out.push(value);
    else run.push(n);
  }
  flush();
  return out.join(', ');
}

/**
 * The flat parse as a tree of loops. A step deeper in `passes` than the one
 * before it ran inside the loop that step opened, so the head is the last step
 * at the outer depth.
 */
export function gather(steps: readonly Line[]): Gathered[] {
  return level(steps, 0, 0);
}

/** `steps` begins at index `first` of the whole parse. */
function level(steps: readonly Line[], depth: number, first: number): Gathered[] {
  const out: Gathered[] = [];
  let open: { at: number; head: Line; body: Line[] } | undefined;
  const close = () => {
    if (!open) return;
    const { at, head, body } = open;
    out.push(body.length ? loop(at, head, body, depth) : { kind: 'step', at, step: head });
  };
  for (const [n, step] of steps.entries()) {
    if (open && step.passes.length > depth) {
      open.body.push(step);
      continue;
    }
    close();
    open = { at: first + n, head: step, body: [] };
  }
  close();
  return out;
}

interface Run {
  readonly pass: Pass;
  /** The first step's index in the whole parse. */
  readonly first: number;
  readonly steps: Line[];
}

function loop(at: number, head: Line, body: readonly Line[], depth: number): Gathered {
  const runs = split(body, depth, at + 1);
  // The shape is the most common one, so one odd first pass cannot make every
  // other pass a misfit.
  const groups = new Map<string, Run[]>();
  for (const run of runs) {
    const key = JSON.stringify(run.steps.map((step) => structure(step, depth)));
    groups.set(key, [...(groups.get(key) ?? []), run]);
  }
  const largest = [...groups.values()].reduce((a, b) => (b.length > a.length ? b : a));
  const [reference] = largest;
  if (!reference) throw new Error('a loop with no passes');
  const { kept, template } = fit(largest, depth);
  const misfits = runs.filter((run) => !kept.has(run));
  return {
    kind: 'loop',
    at,
    head,
    name: reference.pass.name,
    passes: runs.length,
    values: runs.filter((run) => kept.has(run)).map((run) => run.pass.value),
    shape: level(template, depth + 1, reference.first),
    misfits: misfits.map((run) => ({
      at: run.pass.at,
      value: run.pass.value,
      nodes: level(run.steps, depth + 1, run.first),
    })),
  };
}

/** A loop's body steps, cut wherever the pass at this depth changes. */
function split(body: readonly Line[], depth: number, first: number): Run[] {
  const runs: Run[] = [];
  for (const [n, step] of body.entries()) {
    const pass = step.passes[depth];
    if (!pass) throw new Error('a loop body step outside every pass');
    const last = runs.at(-1);
    if (last?.pass.at === pass.at) last.steps.push(step);
    else runs.push({ pass, first: first + n, steps: [step] });
  }
  return runs;
}

/**
 * What two passes must share exactly for one to stand for the other. Not whether
 * each step is reached: only a loop's last pass reports its status, so its `&&`
 * reads differently from every other pass's — see [[weakest]].
 */
function structure(step: Line, depth: number): unknown {
  return [
    step.kind,
    step.key,
    step.depth,
    step.argv.length,
    step.concept === undefined,
    step.cwd === undefined,
    step.host === undefined,
    step.uses.map((used) => [used.write, used.host === undefined]),
    step.passes.slice(depth + 1),
  ];
}

/** Every string on a step a value can appear in, in a fixed order. */
function strings(step: Line): string[] {
  return [
    ...step.argv,
    step.says,
    step.concept ?? '',
    step.cwd ?? '',
    step.host ?? '',
    ...step.uses.flatMap((used) => [used.path, used.host ?? '']),
  ];
}

/** [[strings]] put back: `like` gives which fields are absent. */
function rebuilt(like: Line, from: readonly string[]): Line {
  let n = 0;
  const next = () => from[n++] ?? '';
  const argv = like.argv.map(next);
  const says = next();
  const concept = next();
  const cwd = next();
  const host = next();
  return {
    ...like,
    argv,
    says,
    concept: like.concept === undefined ? undefined : concept,
    cwd: like.cwd === undefined ? undefined : cwd,
    host: like.host === undefined ? undefined : host,
    uses: like.uses.map((used) => {
      const path = next();
      const away = next();
      return { ...used, path, host: used.host === undefined ? undefined : away };
    }),
  };
}

/**
 * The body as one template, and the passes it stands for. Each string is the
 * same in every pass, or the same once each pass's value is put in its slot; a
 * pass no choice of slot fits is left out, for the sheet to draw as it ran.
 */
function fit(runs: readonly Run[], depth: number): { kept: Set<Run>; template: Line[] } {
  const [reference] = runs;
  if (!reference) throw new Error('fitting no passes');
  const marker = slot(depth, reference.pass.name);
  let kept = [...runs];
  const template = reference.steps.map((step, at) => {
    const own = strings(step);
    const chosen = own.map((text, field) => {
      const across = (run: Run) => ({
        text: strings(run.steps[at] ?? step)[field] ?? '',
        value: run.pass.value,
      });
      const fits = (candidate: string) => (run: Run) => {
        const { text: theirs, value } = across(run);
        return candidate.replaceAll(marker, value) === theirs;
      };
      const options = templates(text, reference.pass.value, marker);
      const best = options
        .map((candidate) => ({ candidate, fitting: kept.filter(fits(candidate)) }))
        .reduce((a, b) => (b.fitting.length > a.fitting.length ? b : a));
      kept = best.fitting;
      return best.candidate;
    });
    return { step, chosen };
  });
  return {
    kept: new Set(kept),
    template: template.map(({ step, chosen }, at) => weakest(rebuilt(step, chosen), kept, at)),
  };
}

const CLAIM: readonly Reach[] = ['always', 'on-success', 'sometimes'];

/** A template step claiming no more than any pass it stands for does. */
function weakest(step: Line, runs: readonly Run[], at: number): Line {
  const peers = runs.map((run) => run.steps[at] ?? step);
  const least = (reaches: readonly Reach[]) =>
    CLAIM[Math.max(...reaches.map((reach) => CLAIM.indexOf(reach)))] ?? 'sometimes';
  return {
    ...step,
    reached: least(peers.map((peer) => peer.reached)),
    uses: step.uses.map((used, n) => {
      const theirs = peers.map((peer) => peer.uses[n] ?? used);
      return {
        ...used,
        reached: least(theirs.map((use) => use.reached)),
        certain: theirs.every((use) => use.certain),
      };
    }),
  };
}

/**
 * The ways `value` can have reached `text`, most likely first: not at all, at
 * every place it appears, or at one place. Never inside an outer loop's slot.
 */
function templates(text: string, value: string, marker: string): string[] {
  const taken = [...text.matchAll(MARKED)].map((m) => [m.index, m.index + m[0].length]);
  const free: number[] = [];
  for (let at = text.indexOf(value); at >= 0; at = text.indexOf(value, at + value.length)) {
    if (!taken.some(([from, to]) => at < (to ?? 0) && at + value.length > (from ?? 0)))
      free.push(at);
  }
  const at = (places: readonly number[]) =>
    places.reduceRight(
      (out, place) => out.slice(0, place) + marker + out.slice(place + value.length),
      text,
    );
  const options = [text];
  if (free.length) options.push(at(free));
  if (free.length > 1) options.push(...free.map((place) => at([place])));
  return options;
}
