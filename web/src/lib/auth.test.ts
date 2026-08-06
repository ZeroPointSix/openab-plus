import { describe, expect, it } from 'vitest';
import { loginPathFor, returnToFromSearch } from './auth';

describe('authentication return paths', () => {
  it('preserves the requested session route through login', () => {
    const loginPath = loginPathFor({
      pathname: '/sessions/slack%3A1700.1',
      search: '?view=timeline',
      hash: '',
    });

    expect(loginPath).toBe(
      '/login?return_to=%2Fsessions%2Fslack%253A1700.1%3Fview%3Dtimeline',
    );
    expect(returnToFromSearch(loginPath.split('?')[1])).toBe(
      '/sessions/slack%3A1700.1?view=timeline',
    );
  });

  it('falls back to overview without a return target', () => {
    expect(returnToFromSearch('')).toBe('/overview');
  });

  it('rejects external and protocol-relative return targets', () => {
    expect(
      returnToFromSearch('?return_to=' + encodeURIComponent('https://evil.example')),
    ).toBe('/overview');
    expect(
      returnToFromSearch('?return_to=' + encodeURIComponent('//evil.example')),
    ).toBe('/overview');
  });

  it('rejects backslash host variants', () => {
    expect(
      returnToFromSearch('?return_to=' + encodeURIComponent('/\\evil.example')),
    ).toBe('/overview');
  });
});
