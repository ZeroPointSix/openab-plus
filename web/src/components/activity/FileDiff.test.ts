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

  it('keeps every sparse changed region visible in a large file', () => {
    const before = Array.from({ length: 1000 }, (_, index) => `line ${index + 1}`);
    const after = [...before];
    after[99] = 'line 100 changed';
    after[499] = 'line 500 changed';
    after[899] = 'line 900 changed';

    const preview = buildHunkPreview(before.join('\n'), after.join('\n'));

    for (const line of [100, 500, 900]) {
      expect(preview).toContainEqual(
        expect.objectContaining({
          kind: 'removed',
          oldNumber: line,
          text: `line ${line}`,
        }),
      );
      expect(preview).toContainEqual(
        expect.objectContaining({
          kind: 'added',
          newNumber: line,
          text: `line ${line} changed`,
        }),
      );
    }
  });

  it('bounds previews with hundreds of sparse changed hunks', () => {
    const before = Array.from({ length: 5_001 }, (_, index) => `line ${index + 1}`);
    const after = [...before];
    for (let index = 0; index < 500; index += 1) {
      after[index * 10 + 5] = `changed ${index + 1}`;
    }

    const preview = buildHunkPreview(before.join('\n'), after.join('\n'));
    const changed = preview.filter(
      ({ kind }) => kind === 'removed' || kind === 'added',
    );

    expect(changed.length).toBeLessThanOrEqual(240);
    expect(preview.length).toBeLessThan(2_000);
    expect(preview).toContainEqual(
      expect.objectContaining({
        kind: 'omitted',
        text: expect.stringContaining('changed hunks omitted'),
      }),
    );
  });

  it('does not render an unbounded preview for an unchanged snapshot', () => {
    const content = Array.from({ length: 10_000 }, (_, index) => `line ${index + 1}`).join('\n');

    expect(buildHunkPreview(content, content)).toEqual([]);
  });
});
