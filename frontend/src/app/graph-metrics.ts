import type { GraphNode, Usage } from './models';

/**
 * What the size of a dot means. One control, where there were four toggles.
 *
 * A link graph can show how *connected* a memory is and nothing else — so a
 * rule cited by six projects and never once consulted looks exactly like the
 * one that governs every session. These are the questions the links cannot
 * answer, and each is a different reading of one picture rather than a
 * different picture.
 */
export type Metric = 'standing' | 'use' | 'reads' | 'edits' | 'reach' | 'age' | 'links';

/**
 * Projects at which `reach` is full size.
 *
 * Set from the observed tail rather than a round number: 99.2% of the corpus
 * reaches fourteen projects or fewer, so this spends the whole visible range
 * on the distinction that exists and clamps only the three outliers.
 */
export const REACH_FULL = 14;

/**
 * Each signal's own curve, in 0..1: the one place its scaling is decided, so
 * `standing` composes the same curves the single metrics draw.
 */
export const curve = {
  /** Links in and out. */
  links: (degree: number): number => Math.min(1, Math.sqrt(degree) / 5.5),
  /**
   * Sessions, not turns. A memory hammered for one week and never touched again
   * would otherwise outrank one consulted quietly every week.
   */
  use: (sessions: number): number => Math.min(1, Math.sqrt(sessions) / 3.6),
  /**
   * Deliberate opens, which is a different question from "came up" and has
   * a very different shape: 48% of the live corpus has never been opened at
   * all, and 31 memories come up in five or more sessions without one. That
   * half-empty picture IS the answer — a memory the work only ever mentions
   * in passing is one nobody has checked.
   */
  reads: (reads: number): number => Math.min(1, Math.sqrt(reads) / 5.5),
  edits: (edits: number): number => Math.min(1, Math.log10(1 + edits) / 2.1),
};

/**
 * How big a memory should be drawn, in 0..1, under the current reading.
 *
 * May be zero, but a zero still draws: the radius has a floor of 1.6 world
 * units before this scales it, because a memory that has never been used,
 * never edited and links to nothing is still there, and shrinking it to
 * invisibility would answer the question by hiding it. It draws small, which
 * is the honest depiction.
 *
 * `metric` is the reading asked for. It falls back to links whenever nothing has
 * been mined (`minedAny`), so the picture on a machine with no transcripts is
 * the old one rather than a field of identical dots.
 */
export function weight(
  node: GraphNode,
  usage: Usage | undefined,
  metric: Metric,
  minedAny: boolean,
  newest: number,
): number {
  const reading = minedAny ? metric : 'links';
  if (reading === 'links') return curve.links(node.in_degree + node.out_degree);
  if (!usage) return 0.05;
  switch (reading) {
    case 'use':
      return curve.use(usage.sessions);
    case 'reads':
      return curve.reads(usage.reads);
    case 'edits':
      return curve.edits(usage.edits);
    case 'reach':
      return reach(usage);
    case 'standing':
      return standing(node, usage, newest);
    case 'age':
      return freshness(usage, newest);
  }
}

/**
 * How many separate projects consult this memory, normalised.
 *
 * A different question from how *often* it is used, and the two disagree in
 * the way that matters: a memory hammered all week inside one repository is
 * local knowledge, while one consulted twice in each of six projects is a
 * rule the whole body of work leans on. Only the second should read as
 * load-bearing.
 *
 * **Scaled from one project, not from zero, and saturating at fourteen.**
 * The first version was `sqrt(n) / sqrt(6)` — chosen by analogy with the
 * other metrics without looking at the distribution, and wrong twice over.
 * A single-project memory scored 0.41, so the thing the metric exists to
 * call small was drawn at nearly half size; and everything from 6 projects
 * to 19 clamped to exactly 1.0, flattening 19% of the corpus — including
 * every genuinely cross-cutting rule — into one indistinguishable value.
 *
 * Measured on the live corpus (367 memories): 65 reach one project, 73 two,
 * 63 three, 52 four, and the tail runs 5..19 with only three above 14.
 * Subtracting one puts "used in exactly one place" at zero, where the radius
 * floor still draws it; saturating at 14 keeps the whole meaningful range
 * spread instead of spending it all below six.
 */
export function reach(usage: Usage): number {
  const count = Object.keys(usage.projects).length;
  // Zero and one are the same statement — this memory reaches nowhere else.
  if (count <= 1) return 0;
  return Math.min(1, (Math.sqrt(count) - 1) / (Math.sqrt(REACH_FULL) - 1));
}

/**
 * Freshness against the most recent use anywhere in the corpus rather than
 * against today (`newest`, epoch ms) — the transcripts have an end date, and scoring against now
 * would fade the whole picture uniformly as they age.
 */
export function freshness(usage: Usage, newest: number): number {
  if (!usage.last || newest === 0) return 0.05;
  const days = (newest - Date.parse(usage.last)) / 86_400_000;
  return Math.max(0.05, 1 - Math.min(1, days / 30));
}

/**
 * One number for how load-bearing a memory actually is.
 *
 * The other metrics each answer one question and can each be gamed by a
 * single accident of how the work happened to go — a memory rewritten five
 * times in one afternoon looks busy under `edits`, and one injected into
 * every session looks universal under `use` without anyone ever having read
 * it. This combines the independent signals so that no single accident can
 * carry a memory to the top: it has to come up across sessions, be opened on
 * purpose, matter in more than one project, and still be current.
 *
 * The weights are a judgement, not a measurement, so they are stated rather
 * than buried: breadth of use and deliberate opening carry the most, reach
 * across projects next, freshness least — a rule can be entirely correct and
 * simply not have come up lately, and that is not a demotion.
 *
 * Deliberately NOT the only view. Every input remains selectable on its own,
 * because a composite that cannot be taken apart is one nobody can argue
 * with when it is wrong.
 */
export function standing(node: GraphNode, usage: Usage, newest: number): number {
  return Math.min(
    1,
    0.3 * curve.use(usage.sessions) +
      0.25 * curve.reads(usage.reads) +
      0.2 * reach(usage) +
      0.15 * curve.links(node.in_degree + node.out_degree) +
      0.1 * freshness(usage, newest),
  );
}
