/**
 * The one tool whose approval is an answer rather than a permission.
 *
 * `AskUserQuestion` is gated by `can_use_tool` like every other tool, so it
 * arrives as an ordinary `ask` — but allowing it unchanged is not answering it.
 * The tool reads `answers` out of its own arguments and formats them; it prompts
 * nobody. So a client answers by approving an input it has written the choice
 * into, which is what `updatedInput` is for. Until this console did that, every
 * question it was shown came back to the session as *"The user did not answer
 * the questions."*
 */
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

/** What was chosen: the question's own text against the label, or labels,
 *  picked. The CLI matches these against what it offered, so they go back
 *  verbatim rather than by index. */
export type Answers = Record<string, string | string[]>;

/**
 * The questions in a tool call's arguments, or nothing if they cannot be read.
 *
 * ⚠ **All or nothing, deliberately.** A half-read question would show fewer
 * options than were offered, and a person choosing from a list cannot tell that
 * an option is missing — they would answer a question nobody asked. Failing
 * whole is not a dead end either: a caller that gets nothing back falls to the
 * ordinary allow/refuse row, which still lets the session move.
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
    // Anything other than an explicit `true` is a single choice: guessing the
    // other way would let one tap answer a question that wanted several.
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

/** A narrowing rather than an assertion: everything read here arrives as JSON
 *  off a socket, and `as` would let a wrong guess through silently. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Whether every question has something chosen — what the send button waits for. */
export function complete(questions: readonly Question[], chosen: Answers): boolean {
  return questions.every((q) => {
    const answer = chosen[q.question];
    return Array.isArray(answer) ? answer.length > 0 : typeof answer === 'string';
  });
}
