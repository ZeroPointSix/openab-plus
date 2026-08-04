import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  confirmNavigation,
  discardUnsavedChanges,
  hasUnsavedChanges,
  setNavigationDirty,
} from './navigationGuard';

describe('navigation guard', () => {
  beforeEach(discardUnsavedChanges);

  it('allows navigation when there are no pending edits', () => {
    const confirm = vi.fn(() => false);
    expect(confirmNavigation(confirm)).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
  });

  it('keeps the dirty state when the user stays on the page', () => {
    setNavigationDirty(true);
    expect(confirmNavigation(() => false)).toBe(false);
    expect(hasUnsavedChanges()).toBe(true);
  });

  it('clears the dirty state after discard confirmation', () => {
    setNavigationDirty(true);
    expect(confirmNavigation(() => true)).toBe(true);
    expect(hasUnsavedChanges()).toBe(false);
  });
});
