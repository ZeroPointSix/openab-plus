import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  cancelledHistoryDelta,
  confirmNavigation,
  confirmRouteNavigation,
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

  it('does not consume dirty state for the current route', () => {
    setNavigationDirty(true);
    const confirm = vi.fn(() => true);

    expect(confirmRouteNavigation('/config', '/config', confirm)).toBe(false);
    expect(confirm).not.toHaveBeenCalled();
    expect(hasUnsavedChanges()).toBe(true);
  });

  it('restores cancelled history moves in either direction', () => {
    expect(cancelledHistoryDelta(5, { idx: 4 })).toBe(1);
    expect(cancelledHistoryDelta(5, { idx: 6 })).toBe(-1);
    expect(cancelledHistoryDelta(undefined, { idx: 4 })).toBeUndefined();
  });

  it('clears the dirty state after discard confirmation', () => {
    setNavigationDirty(true);
    expect(confirmNavigation(() => true)).toBe(true);
    expect(hasUnsavedChanges()).toBe(false);
  });
});
