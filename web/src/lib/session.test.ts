import { describe, expect, it } from 'vitest';
import {
  applySessionEvent,
  filterSessions,
  matchesSessionKeyword,
  mergeTimelineItem,
  parseSessionEventPayload,
  sessionListSubtitle,
  sessionListTitle,
  sessionMetrics,
  sessionProjectGroup,
  sortSessions,
  timelineItemFromEvent,
  titleFromTranscript,
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
  it('prefers a readable task title over the raw session id', () => {
    expect(sessionListTitle({ ...sessions[0], title: '修复列表标题' })).toBe(
      '修复列表标题',
    );
    expect(sessionListTitle(sessions[0])).toBe('Primary');
    expect(sessionListTitle(sessions[0], '  优化前端效果  ')).toBe(
      '优化前端效果',
    );
    expect(sessionListTitle(sessions[1])).toBe('Claude · beta');
  });

  it('keeps same-profile sessions distinguishable by their list titles', () => {
    const first = {
      ...sessions[0],
      session_id: 'slack:task-a',
      title: '优化前端效果',
      profile_name: 'Primary',
    };
    const second = {
      ...sessions[0],
      session_id: 'slack:task-b',
      title: '修复回归缺陷',
      profile_name: 'Primary',
    };

    // Mirrors SessionSidebar: only the active row gets a live transcript title.
    expect(sessionListTitle(first, undefined)).toBe('优化前端效果');
    expect(sessionListTitle(second, undefined)).toBe('修复回归缺陷');
    expect(sessionListTitle(first, '优化前端效果')).toBe('优化前端效果');
    expect(sessionListTitle(second, undefined)).not.toBe(
      sessionListTitle(first, undefined),
    );
  });

  it('describes the source on the subtitle instead of repeating the id', () => {
    expect(sessionListSubtitle(sessions[0])).toBe('Slack · alpha');
  });

  it('derives a session title from the first user turn', () => {
    expect(
      titleFromTranscript([
        {
          entry_id: 'e0',
          sequence: 0,
          timestamp: '2026-08-02T10:00:00Z',
          role: 'system',
          content: 'boot',
        },
        {
          entry_id: 'e1',
          sequence: 1,
          timestamp: '2026-08-02T10:00:01Z',
          role: 'user',
          content: '  优化前端效果\n第二行应当忽略  ',
        },
      ]),
    ).toBe('优化前端效果');
    expect(titleFromTranscript([])).toBeUndefined();
  });

  it('groups sessions by project directory rather than agent type', () => {
    expect(sessionProjectGroup(sessions[0])).toBe('alpha');
    expect(sessionProjectGroup({ ...sessions[0], workdir: '' })).toBe(
      '未归类项目',
    );
  });

  it('uses 空闲 for idle sessions instead of 等待中', () => {
    expect(sessionStatusDisplay.idle.label).toBe('空闲');
  });

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

  it('matches keywords across session identity and model metadata', () => {
    const modeled = {
      ...sessions[0],
      model: 'claude-opus-5',
      title: '修复列表标题',
    };

    expect(matchesSessionKeyword(modeled, 'opus')).toBe(true);
    expect(matchesSessionKeyword(modeled, '列表标题')).toBe(true);
    expect(matchesSessionKeyword(modeled, '/workspace/alpha')).toBe(true);
    expect(matchesSessionKeyword(modeled, 'discord')).toBe(false);
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

  it('sorts by the instant represented by timestamps across time zones', () => {
    const [newest, oldest] = sortSessions([
      {
        ...sessions[0],
        session_id: 'slack:newest',
        updated_at: '2026-08-12T06:43:22Z',
      },
      {
        ...sessions[1],
        session_id: 'discord:oldest',
        updated_at: '2026-08-12T14:42:22+08:00',
      },
    ]);

    expect(newest.session_id).toBe('slack:newest');
    expect(oldest.session_id).toBe('discord:oldest');
  });

  it('upserts a snapshot received from SSE without dropping its list title', () => {
    const titledSessions = [
      { ...sessions[0], title: '优化前端效果' },
      sessions[1],
    ];
    const next = applySessionEvent(titledSessions, {
      sequence: 42,
      event: 'status_changed',
      snapshot: { ...sessions[0], status: 'idle' },
    });
    const updated = next.find((item) => item.session_id === 'slack:alpha');
    expect(updated?.status).toBe('idle');
    expect(updated?.title).toBe('优化前端效果');
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
