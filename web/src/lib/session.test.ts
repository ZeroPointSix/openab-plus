import { describe, expect, it } from 'vitest';
import {
  applySessionEvent,
  filterSessions,
  initialTimeline,
  matchesSessionKeyword,
  parseSessionEventPayload,
  sessionMetrics,
  timelineItemFromEvent,
  visibleTimelineItems,
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

  it('filters by Agent type before rendering grouped history', () => {
    const matching = filterSessions(sessions, { agent: 'claude' });

    expect(matching).toHaveLength(1);
    expect(matching[0].session_id).toBe('discord:beta');
  });

  it('matches keywords across session identity fields', () => {
    expect(matchesSessionKeyword(sessions[0], 'ALPHA')).toBe(true);
    expect(matchesSessionKeyword(sessions[0], 'primary')).toBe(true);
    expect(matchesSessionKeyword(sessions[0], 'not-found')).toBe(false);
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

  it('drops seed timeline entries once replayed stream events arrive', () => {
    const seeds = initialTimeline(sessions[0]);
    expect(seeds.length).toBeGreaterThan(0);
    // 只有种子条目时原样展示（SSE 尚未补齐历史时的兜底）。
    expect(visibleTimelineItems(seeds)).toEqual(seeds);

    const replayed = timelineItemFromEvent(
      { sequence: 7, event: 'session.created', snapshot: sessions[0] },
      'gen:7',
    );
    expect(replayed).not.toBeNull();
    // 真实的流事件（带 sequence）到达后，种子条目被过滤，避免重复。
    expect(visibleTimelineItems([...seeds, replayed!])).toEqual([replayed]);
  });
});
