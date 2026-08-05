const UNSAVED_MESSAGE = '当前配置还有未保存的修改，确定要离开吗？';

let navigationDirty = false;

export function setNavigationDirty(value: boolean) {
  navigationDirty = value;
}

export function hasUnsavedChanges(): boolean {
  return navigationDirty;
}

export function discardUnsavedChanges() {
  navigationDirty = false;
}

export function confirmNavigation(
  confirmFn: (message: string) => boolean = (message) => window.confirm(message),
): boolean {
  if (!navigationDirty) return true;
  if (!confirmFn(UNSAVED_MESSAGE)) return false;
  navigationDirty = false;
  return true;
}


export function confirmRouteNavigation(
  currentPath: string,
  targetPath: string,
  confirmFn?: (message: string) => boolean,
): boolean {
  if (currentPath === targetPath) return false;
  return confirmNavigation(confirmFn);
}

export function historyIndex(state: unknown): number | undefined {
  if (!state || typeof state !== 'object') return undefined;
  const index = (state as { idx?: unknown }).idx;
  return typeof index === 'number' && Number.isInteger(index) ? index : undefined;
}

export function cancelledHistoryDelta(
  currentIndex: number | undefined,
  nextState: unknown,
): number | undefined {
  const nextIndex = historyIndex(nextState);
  if (currentIndex === undefined || nextIndex === undefined) return undefined;
  const delta = currentIndex - nextIndex;
  return delta === 0 ? undefined : delta;
}
