export const ADMIN_TOKEN_KEY = 'openab.adminToken';
export const LAST_EVENT_ID_KEY = 'openab.lastEventId';
export const UNAUTHORIZED_EVENT = 'openab:unauthorized';

export function readAdminToken(): string {
  const persisted = localStorage.getItem(ADMIN_TOKEN_KEY)?.trim() || '';
  if (persisted) {
    return persisted;
  }

  const legacy = sessionStorage.getItem(ADMIN_TOKEN_KEY)?.trim() || '';
  if (legacy) {
    localStorage.setItem(ADMIN_TOKEN_KEY, legacy);
    sessionStorage.removeItem(ADMIN_TOKEN_KEY);
  }
  return legacy;
}

export function saveAdminToken(token: string): void {
  localStorage.setItem(ADMIN_TOKEN_KEY, token.trim());
  sessionStorage.removeItem(ADMIN_TOKEN_KEY);
}

export function clearAdminToken(): void {
  localStorage.removeItem(ADMIN_TOKEN_KEY);
  sessionStorage.removeItem(ADMIN_TOKEN_KEY);
  sessionStorage.removeItem(LAST_EVENT_ID_KEY);
}

interface RouteLocation {
  pathname: string;
  search: string;
  hash: string;
}

export function loginPathFor(location: RouteLocation): string {
  const returnTo = location.pathname + location.search + location.hash;
  return '/login?return_to=' + encodeURIComponent(returnTo);
}

export function returnToFromSearch(search: string): string {
  const candidate = new URLSearchParams(search).get('return_to');
  if (!candidate) {
    return '/overview';
  }

  // Allow only same-app HashRouter paths. In particular, reject protocol-
  // relative URLs and backslash variants that browsers can normalize to hosts.
  if (
    !candidate.startsWith('/') ||
    candidate.startsWith('//') ||
    candidate.includes('\\')
  ) {
    return '/overview';
  }
  return candidate;
}

export function notifyUnauthorized(): void {
  clearAdminToken();
  window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT));
}
