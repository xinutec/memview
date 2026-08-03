import { describe, expect, it } from 'vitest';

import { modelName } from './model';

describe('model names', () => {
  it('drops the bracketed variant, which the window beside it already states', () => {
    expect(modelName('claude-opus-5[1m]')).toBe('Opus 5');
  });

  it('reads a hyphenated version as the decimal it is', () => {
    // Every one of these is in the transcript corpus.
    expect(modelName('claude-opus-4-8')).toBe('Opus 4.8');
    expect(modelName('claude-sonnet-4-6')).toBe('Sonnet 4.6');
    expect(modelName('claude-fable-5')).toBe('Fable 5');
  });

  it('drops a build date, which is not part of the name anybody says', () => {
    expect(modelName('claude-haiku-4-5-20251001')).toBe('Haiku 4.5');
  });

  it('names a bare alias too', () => {
    // `opus`, `sonnet` and `haiku` all appear without the prefix or a version.
    expect(modelName('opus')).toBe('Opus');
    expect(modelName('haiku')).toBe('Haiku');
  });

  it('returns an unrecognised id untouched rather than half-parsing it', () => {
    // ⚠ New models arrive between releases. A header confidently naming one it
    // has never seen is worse than one showing the id — the id is at least true,
    // and it is the thing worth reporting when the name looks wrong.
    expect(modelName('claude-newthing-9')).toBe('claude-newthing-9');
    // The CLI's own sentinel for a message no model produced.
    expect(modelName('<synthetic>')).toBe('<synthetic>');
  });

  it('says nothing when the session has not said', () => {
    expect(modelName(undefined)).toBeUndefined();
  });
});
