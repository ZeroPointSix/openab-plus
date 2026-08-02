import { describe, expect, it } from 'vitest';
import {
  applySessionEvent,
  filterSessions,
  parseSessionEventPayload,
  sessionMetrics,
} from './session';
import { SessionSnapshot } from '../types';

const sessions: SessionSnapshot[] = [
  {
    session_id: 'slack:alpha',
    agent: 'codex',
    source: { platform: 'slack', thread_id: 'alpha' },
    workdir: '/workspace/alpha',
    profile_id: 'primary',
    profile_name: 'Primary',
    status: 'running',
    created_at: '2026-08-02T10:00:00Z',
    updated_at: '2026-08-02T10:10:00Z',
  },
  {
    session_id: 'discord:beta',
    agent: 'claude',
    source: { platform: 'discord', thread_id: 'beta' },
    workdir: '/workspace/beta',
    status: 'error',
    last_error: 'process exited',
    created_at: '2026-08-02T09:00:00Z',
    updated_at: '2026-08-02T09:05:00Z',
  },
];

describe('session helpers', () => {
  it('filters by platform and profile', () => {
    expect(
      filterSessions(sessions, { platform: 'slack', profile: 'primary' }),
    ).toHaveLength(1);
  });

  it('computes dashboard metrics', () => {
    expect(sessionMetrics(sessions)).toEqual({
      total: 2,
      active: 1,
      running: 1,
      failed: 1,
    });
  });

  it('upserts a snapshot received from SSE', () => {
    const next = applySessionEvent(sessions, {
      sequence: 42,
      event: 'status_changed',
      snapshot: { ...sessions[0], status: 'idle' },
    });
    expect(next.find((item) => item.session_id === 'slack:alpha')?.status).toBe(
      'idle',
    );
  });

  it('distinguishes a session error event from a stream diagnostic', () => {
    const event = parseSessionEventPayload(
      JSON.stringify({
        sequence: 43,
        event: 'error',
        snapshot: { ...sessions[0], status: 'error' },
      }),
    );

    expect(event?.snapshot?.status).toBe('error');
    expect(
      parseSessionEventPayload(
        JSON.stringify({ error: 'event history unavailable' }),
      ),
    ).toBeNull();
  });
});
