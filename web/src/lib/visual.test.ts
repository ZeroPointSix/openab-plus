import { describe, expect, it } from 'vitest';
import { visualForEntity } from './visual';

describe('visualForEntity', () => {
  it('is deterministic for the same name', () => {
    expect(visualForEntity('codex')).toEqual(visualForEntity('codex'));
  });

  it('uppercases the first alphanumeric character', () => {
    expect(visualForEntity('codex').initial).toBe('C');
    expect(visualForEntity('  claude').initial).toBe('C');
  });

  it('falls back to a placeholder for empty names', () => {
    expect(visualForEntity('').initial).toBe('?');
    expect(visualForEntity(undefined).initial).toBe('?');
    expect(visualForEntity('---').initial).toBe('?');
  });

  it('keeps distinct colors inside the palette', () => {
    const names = ['codex', 'claude', 'gemini', 'opencode', 'kiro'];
    for (const name of names) {
      const visual = visualForEntity(name);
      expect(visual.color).toMatch(/^#[0-9a-f]{6}$/);
      expect(visual.background).toMatch(/^#[0-9a-f]{6}$/);
    }
  });
});
