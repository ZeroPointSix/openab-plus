import {
  SessionEventPayload,
  SessionFilters,
  SessionSnapshot,
  SessionTimelineItem,
} from '../types';
const ACTIVE = new Set(['starting', 'idle', 'running', 'suspended']);
const RUNNING = new Set(['starting', 'running']);
const FAILED = new Set(['error', 'exited']);

export function parseSessionEventPayload(
  data: string,
): SessionEventPayload | null {
  try {
    const value = JSON.parse(data) as Partial<SessionEventPayload>;
    if (
      !Number.isInteger(value.sequence) ||
      typeof value.event !== 'string' ||
      !value.snapshot ||
      typeof value.snapshot.session_id !== 'string'
    ) {
      return null;
    }
    return value as SessionEventPayload;
  } catch {
    return null;
  }
}

function sessionTimestamp(session: SessionSnapshot): number {
  const timestamp = Date.parse(session.updated_at || session.created_at);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

export function sortSessions(sessions: SessionSnapshot[]): SessionSnapshot[] {
  return [...sessions].sort(
    (a, b) => sessionTimestamp(b) - sessionTimestamp(a),
  );
}

export function matchesSessionKeyword(
  session: SessionSnapshot,
  keyword: string,
): boolean {
  const normalized = keyword.trim().toLocaleLowerCase();
  if (!normalized) return true;
  return [
    session.session_id,
    session.agent,
    session.profile_id,
    session.profile_name,
    session.source.platform,
    session.source.thread_id,
    session.workdir,
    session.model,
  ]
    .filter(Boolean)
    .some((value) => String(value).toLocaleLowerCase().includes(normalized));
}

export function filterSessions(
  sessions: SessionSnapshot[],
  filters: SessionFilters,
): SessionSnapshot[] {
  return sortSessions(sessions).filter((session) => {
    if (filters.platform && session.source.platform !== filters.platform) {
      return false;
    }
    if (filters.status && session.status !== filters.status) return false;
    if (filters.agent && session.agent !== filters.agent) return false;
    if (
      filters.profile &&
      session.profile_id !== filters.profile &&
      session.profile_name !== filters.profile
    ) {
      return false;
    }
    if (filters.updatedRange) {
      const time = new Date(session.updated_at).getTime();
      const start = new Date(filters.updatedRange[0]).getTime();
      const end = new Date(filters.updatedRange[1]).getTime();
      if (time < start || time > end) return false;
    }
    return true;
  });
}

export function sessionMetrics(sessions: SessionSnapshot[]) {
  return {
    total: sessions.length,
    active: sessions.filter((session) => ACTIVE.has(session.status)).length,
    running: sessions.filter((session) => RUNNING.has(session.status)).length,
    failed: sessions.filter((session) => FAILED.has(session.status)).length,
  };
}

export function applySessionEvent(
  current: SessionSnapshot[] | undefined,
  event: SessionEventPayload,
): SessionSnapshot[] {
  const sessions = current ? [...current] : [];
  if (!event.snapshot) return sessions;
  const index = sessions.findIndex(
    (session) => session.session_id === event.snapshot?.session_id,
  );
  if (index === -1) sessions.push(event.snapshot);
  else sessions[index] = event.snapshot;
  return sortSessions(sessions);
}

export function mergeTimelineItem(
  current: SessionTimelineItem[] | undefined,
  timelineItem: SessionTimelineItem,
): SessionTimelineItem[] {
  const timeline = (current || []).filter(
    (item) => !item.id.startsWith('initial:'),
  );
  if (timeline.some((item) => item.id === timelineItem.id)) {
    return timeline;
  }
  return [...timeline, timelineItem].slice(-60);
}

export function timelineItemFromEvent(
  event: SessionEventPayload,
  streamEventId?: string,
): SessionTimelineItem | null {
  if (!event.snapshot) return null;
  return {
    id:
      (streamEventId || String(event.sequence)) + ':' + event.event,
    event: event.event,
    status: event.snapshot.status,
    at: event.snapshot.updated_at || event.snapshot.created_at,
    error: event.snapshot.last_error,
    sequence: event.sequence,
  };
}

export function initialTimeline(
  session: SessionSnapshot,
): SessionTimelineItem[] {
  const items: SessionTimelineItem[] = [
    {
      id: 'initial:created',
      event: 'session.created',
      status: 'idle',
      at: session.created_at,
    },
  ];
  if (
    session.updated_at !== session.created_at ||
    session.status !== 'idle'
  ) {
    items.push({
      id: 'initial:current',
      event: 'current',
      status: session.status,
      at: session.updated_at,
      error: session.last_error,
    });
  }
  return items;
}

/**
 * 冷启动 / 刷新后，SSE 会从 0 补齐历史事件；一旦时间线里出现带 sequence
 * 的真实流事件，就把 initialTimeline 种下的种子条目（created/current）
 * 过滤掉，避免同一条「会话创建 / 当前状态」在时间线上重复出现。
 */
export function visibleTimelineItems(
  items: SessionTimelineItem[],
): SessionTimelineItem[] {
  const hasStreamed = items.some((item) => typeof item.sequence === 'number');
  return hasStreamed
    ? items.filter((item) => typeof item.sequence === 'number')
    : items;
}
