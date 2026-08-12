import { describe, expect, it } from 'vitest';
import { buildHunkPreview } from './FileDiff';

describe('bounded file diff preview', () => {
  it('retains a change near the end of a large file instead of slicing only leading context', () => {
    const before = Array.from({ length: 500 }, (_, index) => `line ${index + 1}`);
    const after = [...before];
    after[399] = 'line 400 changed';

    const preview = buildHunkPreview(before.join('\n'), after.join('\n'));

    expect(preview).toContainEqual(
      expect.objectContaining({ kind: 'removed', oldNumber: 400, text: 'line 400' }),
    );
    expect(preview).toContainEqual(
      expect.objectContaining({ kind: 'added', newNumber: 400, text: 'line 400 changed' }),
    );
    expect(preview.length).toBeLessThan(20);
  });

  it('renders all initial content as additions for a new file', () => {
    expect(buildHunkPreview('', 'first line\nsecond line')).toEqual([
      { kind: 'added', text: 'first line', newNumber: 1 },
      { kind: 'added', text: 'second line', newNumber: 2 },
    ]);
  });

  it('caps a large replacement while preserving both leading and trailing changed lines', () => {
    const before = Array.from({ length: 400 }, (_, index) => `before ${index + 1}`).join('\n');
    const after = Array.from({ length: 400 }, (_, index) => `after ${index + 1}`).join('\n');

    const preview = buildHunkPreview(before, after);

    expect(preview).toContainEqual(
      expect.objectContaining({ kind: 'removed', text: 'before 1' }),
    );
    expect(preview).toContainEqual(
      expect.objectContaining({ kind: 'added', text: 'after 400' }),
    );
    expect(preview).toContainEqual(
      expect.objectContaining({ kind: 'omitted', text: expect.stringContaining('changed lines omitted') }),
    );
  });
});
