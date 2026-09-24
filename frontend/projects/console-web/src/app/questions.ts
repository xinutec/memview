/**
 * The one tool whose approval is an answer rather than a permission.
 * `AskUserQuestion` arrives as an ordinary `ask`, but it reads `answers` out of
 * its own arguments and prompts nobody — so a client answers by approving an
 * input it has written the choice into (`updatedInput`).
 */
import type { Answer } from './generated/Answer';
import type { Question } from './generated/Question';
import type { Reply } from './generated/Reply';

export type { Choice } from './generated/Choice';
export type { Question } from './generated/Question';

/**
 * What was chosen: the question's own text against the label, or labels,
 * picked — verbatim, since the CLI matches them against what it offered.
 */
export type Answers = Record<string, Answer>;

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
