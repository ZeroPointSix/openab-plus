export const ADMIN_TOKEN_KEY = 'openab.adminToken';
export const LAST_EVENT_ID_KEY = 'openab.lastEventId';
export const UNAUTHORIZED_EVENT = 'openab:unauthorized';

export function readAdminToken(): string {
  return sessionStorage.getItem(ADMIN_TOKEN_KEY)?.trim() || '';
}

export function saveAdminToken(token: string): void {
  sessionStorage.setItem(ADMIN_TOKEN_KEY, token.trim());
}

export function clearAdminToken(): void {
  sessionStorage.removeItem(ADMIN_TOKEN_KEY);
  sessionStorage.removeItem(LAST_EVENT_ID_KEY);
}

export function notifyUnauthorized(): void {
  clearAdminToken();
  window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT));
}
