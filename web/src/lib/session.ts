import {
  SessionEventPayload,
  SessionFilters,
  SessionSnapshot,
  SessionTimelineItem,
  TranscriptEntry,
} from '../types';
import {
  agentDisplayName,
  sessionStatusDisplay,
  sourcePlatformLabel,
} from './format';

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

const SESSION_ID_PREVIEW_LENGTH = 8;
const SESSION_TITLE_MAX_LENGTH = 48;

/** Short, readable form of an otherwise opaque session id. */
export function shortSessionId(sessionId: string): string {
  const tail = sessionId.trim().split(':').at(-1) || sessionId.trim();
  return tail.length > SESSION_ID_PREVIEW_LENGTH
    ? tail.slice(0, SESSION_ID_PREVIEW_LENGTH)
    : tail;
}

/**
 * Derive a human task title from the first user turn of a transcript.
 *
 * ZER-715: the list API exposes this derived value on `SessionSnapshot.title`;
 * the transcript helper keeps detail-view derivation consistent with that contract.
 */
export function titleFromTranscript(
  entries: TranscriptEntry[],
): string | undefined {
  const firstUserTurn = entries.find(
    (entry) => entry.role === 'user' && entry.content?.trim(),
  );
  const firstLine = firstUserTurn?.content
    ?.split('\n')
    .map((line) => line.trim())
    .find(Boolean);
  if (!firstLine) return undefined;
  return firstLine.length > SESSION_TITLE_MAX_LENGTH
    ? firstLine.slice(0, SESSION_TITLE_MAX_LENGTH) + '…'
    : firstLine;
}

/** Group sessions by project (working directory) rather than by agent type. */
export function sessionProjectGroup(session: SessionSnapshot): string {
  const workdir = session.workdir?.trim();
  if (!workdir) return '未归类项目';
  const segments = workdir.split(/[/\\]+/).filter(Boolean);
  return segments.at(-1) || workdir;
}

export function sessionListTitle(
  session: SessionSnapshot,
  derivedTitle?: string,
): string {
  const title = derivedTitle?.trim() || session.title?.trim();
  if (title) return title;
  const profileName = session.profile_name?.trim();
  if (profileName) return profileName;
  return (
    agentDisplayName(session.agent) +
    ' · ' +
    shortSessionId(session.session_id)
  );
}

export function sessionListSubtitle(session: SessionSnapshot): string {
  const platform = sourcePlatformLabel(session.source?.platform);
  const thread = session.source?.thread_id?.trim();
  return (
    [platform, thread].filter(Boolean).join(' · ') ||
    shortSessionId(session.session_id)
  );
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
  const query = keyword.trim().toLowerCase();
  if (!query) return true;
  return [
    session.session_id,
    session.agent,
    session.source?.platform,
    session.source?.thread_id,
    session.workdir,
    session.title,
    session.profile_name,
    session.profile_id,
    session.model,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
    .includes(query);
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
    active: sessions.filter((session) => sessionStatusDisplay[session.status].active)
      .length,
    running: sessions.filter(
      (session) => sessionStatusDisplay[session.status].running,
    ).length,
    failed: sessions.filter((session) => sessionStatusDisplay[session.status].failed)
      .length,
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
  else {
    sessions[index] = {
      ...event.snapshot,
      title: event.snapshot.title || sessions[index].title,
    };
  }
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
