import { describe, expect, it } from 'vitest';
import {
  applySessionEvent,
  filterSessions,
  mergeTimelineItem,
  parseSessionEventPayload,
  sessionMetrics,
  timelineItemFromEvent,
} from './session';
import { SessionSnapshot } from '../types';
import { sessionStatusDisplay, sessionStatusOptions } from './format';

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

  it('derives completed state display, filters, and metrics from one mapping', () => {
    const completed: SessionSnapshot = {
      ...sessions[0],
      session_id: 'discord:completed',
      status: 'exited',
    };

    expect(sessionStatusDisplay.exited.label).toBe('已完成');
    expect(sessionStatusOptions).toContainEqual({
      label: '已完成',
      value: 'exited',
    });
    expect(sessionMetrics([...sessions, completed])).toEqual({
      total: 3,
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

  it('replaces temporary detail seeds when real history arrives', () => {
    const initial = [
      {
        id: 'initial:created',
        event: 'session.created',
        status: 'idle' as const,
        at: sessions[0].created_at,
      },
    ];
    const historical = {
      id: 'generation-a:1:session.created',
      event: 'session.created',
      status: 'idle' as const,
      at: sessions[0].created_at,
    };

    expect(mergeTimelineItem(initial, historical)).toEqual([historical]);
    expect(mergeTimelineItem([historical], historical)).toEqual([historical]);
  });

  it('uses the full SSE id to disambiguate timeline events across restarts', () => {
    const event = {
      sequence: 1,
      event: 'status_changed',
      snapshot: { ...sessions[0], status: 'idle' as const },
    };

    const previous = timelineItemFromEvent(event, 'generation-a:1');
    const restarted = timelineItemFromEvent(event, 'generation-b:1');

    expect(previous?.id).toBe('generation-a:1:status_changed');
    expect(restarted?.id).toBe('generation-b:1:status_changed');
    expect(previous?.id).not.toBe(restarted?.id);
  });
});
