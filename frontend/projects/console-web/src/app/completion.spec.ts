import { describe, expect, it } from 'vitest';

import { choicesIn, suggest } from './completion';
import type { Commands } from './models';

/** Shaped like a session's real answer, trimmed. */
const FROM: Commands = {
  commands: [
    {
      name: 'compact',
      description: 'Free up context',
      argument_hint: '<optional custom summarization instructions>',
    },
    { name: 'context', description: 'Show current context usage' },
    {
      name: 'effort',
      description: 'Set effort level',
      argument_hint: '<low|medium|high|xhigh|max|auto>',
    },
    { name: 'model', description: 'Set the AI model', argument_hint: '<model>' },
    { name: 'output-style', description: 'Switch style', argument_hint: '[style]' },
    { name: 'usage', description: 'Show plan usage', aliases: ['cost', 'stats'] },
  ],
  models: [
    {
      value: 'opus',
      display_name: 'Opus 5.5',
      description: 'For complex work',
      effort_levels: ['low', 'max'],
    },
    {
      value: 'fable',
      display_name: 'Fable 5.1',
      description: 'Toughest challenges',
      effort_levels: [],
    },
  ],
  output_styles: ['default', 'Concise'],
};

const labels = (text: string) => suggest(text, FROM).map((s) => s.label);

describe('suggest', () => {
  it('offers every command on a bare slash, and narrows as the name is typed', () => {
    expect(labels('/')).toHaveLength(6);
    expect(labels('/co')).toEqual(['/compact', '/context', '/usage']);
  });

  it('finds a command by its alias, after the ones its name matches', () => {
    expect(labels('/st')).toEqual(['/usage']);
  });

  it('leaves room for the value of a command that takes one, and none for one that does not', () => {
    const [effort] = suggest('/eff', FROM);
    expect(effort?.fill).toBe('/effort ');
    const [context] = suggest('/conte', FROM);
    expect(context?.fill).toBe('/context');
  });

  it('offers the values a hint lists, narrowed by what is typed', () => {
    expect(labels('/effort ')).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'auto']);
    expect(suggest('/effort m', FROM).map((s) => s.fill)).toEqual([
      '/effort medium',
      '/effort max',
    ]);
  });

  it('offers the session’s own models and styles', () => {
    expect(labels('/model ')).toEqual(['opus', 'fable']);
    expect(suggest('/model f', FROM)[0]?.detail).toContain('Fable 5.1');
    expect(labels('/output-style ')).toEqual(['default', 'Concise']);
  });

  it('says what free text a command takes, as a row that cannot be picked', () => {
    const [row] = suggest('/compact ', FROM);
    expect(row?.fill).toBeUndefined();
    expect(row?.label).toBe('<optional custom summarization instructions>');
  });

  it('offers nothing once what is typed is already complete', () => {
    // A picked value would otherwise be offered again, and the list reopen.
    expect(suggest('/effort max', FROM)).toEqual([]);
    expect(suggest('/context', FROM)).toEqual([]);
  });

  it('offers nothing for words, an unknown command, or before the session has answered', () => {
    expect(suggest('hello', FROM)).toEqual([]);
    expect(suggest('/nope ', FROM)).toEqual([]);
    expect(suggest('/', undefined)).toEqual([]);
  });
});

describe('choicesIn', () => {
  it('reads the CLI’s hint notation', () => {
    expect(choicesIn('[on|off]')).toEqual(['on', 'off']);
    expect(choicesIn('[auto|<tokens>]')).toEqual(['auto']);
    expect(choicesIn('[reconnect|enable|disable [<server>|all]]')).toEqual([
      'reconnect',
      'enable',
      'disable',
    ]);
    expect(choicesIn('[name]')).toEqual([]);
    expect(choicesIn('key=value')).toEqual([]);
    expect(choicesIn('[--request-id <uuid>] (--wizard | --fix)')).toEqual([]);
  });
});
