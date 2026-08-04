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
