/**
 * The one tool whose approval is an answer rather than a permission.
 * `AskUserQuestion` arrives as an ordinary `ask`, but it reads `answers` out of
 * its own arguments and prompts nobody — so a client answers by approving an
 * input it has written the choice into (`updatedInput`).
 */
import type { Answer } from './generated/Answer';
import type { Reply } from './generated/Reply';

export const QUESTION_TOOL = 'AskUserQuestion';

/** One thing that could be picked. */
export interface Choice {
  readonly label: string;
  /** What picking it would mean. Often the only part worth reading. */
  readonly description: string;
}

/** One question, as the tool asked it. */
export interface Question {
  readonly question: string;
  /** A word or two naming the decision, for the chip above it. May be empty. */
  readonly header: string;
  readonly multiSelect: boolean;
  readonly options: readonly Choice[];
}

/**
 * What was chosen: the question's own text against the label, or labels,
 * picked — verbatim, since the CLI matches them against what it offered.
 */
export type Answers = Record<string, Answer>;

/**
 * The questions in a tool call's arguments, or nothing if they cannot be read.
 * All or nothing: a half-read question would show fewer options than were
 * offered. A caller that gets nothing falls to the ordinary allow/refuse row.
 */
export function questionsOf(input: unknown): readonly Question[] | undefined {
  if (!isRecord(input)) return undefined;
  const raw = input['questions'];
  if (!Array.isArray(raw) || raw.length === 0) return undefined;
  const read = raw.map(question).filter((q): q is Question => q !== undefined);
  return read.length === raw.length ? read : undefined;
}

function question(value: unknown): Question | undefined {
  if (!isRecord(value)) return undefined;
  const raw = value;
  const asked = raw['question'];
  const options = raw['options'];
  if (typeof asked !== 'string' || asked === '' || !Array.isArray(options)) return undefined;
  const read = options.map(choice).filter((c): c is Choice => c !== undefined);
  if (read.length === 0 || read.length !== options.length) return undefined;
  return {
    question: asked,
    header: typeof raw['header'] === 'string' ? raw['header'] : '',
    // Anything other than an explicit `true` is a single choice.
    multiSelect: raw['multiSelect'] === true,
    options: read,
  };
}

function choice(value: unknown): Choice | undefined {
  if (!isRecord(value)) return undefined;
  const raw = value;
  const label = raw['label'];
  if (typeof label !== 'string' || label === '') return undefined;
  return {
    label,
    description: typeof raw['description'] === 'string' ? raw['description'] : '',
  };
}

/** A narrowing rather than an assertion: everything here arrives as JSON off a socket. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Notes against questions, by the question's own text. */
export type Notes = Record<string, string>;

/** What came back about a question, mirroring `protocol::Reply`. */
export type { Reply };

/**
 * What was said, in one line for the row that records it: the labels alone,
 * since the questions are on screen above. Words win over labels, which is the
 * CLI's own precedence.
 */
export function choiceOf(reply: Reply | undefined): string {
  const said = reply?.response?.trim();
  if (said) return said;
  // Every question answered at all — by a choice, a note, or both; a note alone
  // is an answer the CLI reports as `(no option selected)`.
  const asked = new Set([
    ...Object.keys(reply?.answers ?? {}),
    ...Object.keys(reply?.annotations ?? {}),
  ]);
  return [...asked]
    .map((question) => {
      const answer = reply?.answers?.[question];
      const chose = Array.isArray(answer) ? answer.join(', ') : (answer ?? '');
      const note = reply?.annotations?.[question]?.notes?.trim() ?? '';
      if (chose && note) return `${chose} (${note})`;
      return chose || note;
    })
    .filter((said) => said !== '')
    .join(' · ');
}

/**
 * Whether every question has been answered — what the send button waits for.
 * A note counts: the CLI treats a question carrying only a note as answered.
 */
export function complete(
  questions: readonly Question[],
  chosen: Answers,
  notes: Notes = {},
): boolean {
  return questions.every((q) => {
    if (notes[q.question]?.trim()) return true;
    const answer = chosen[q.question];
    return Array.isArray(answer) ? answer.length > 0 : typeof answer === 'string';
  });
}
