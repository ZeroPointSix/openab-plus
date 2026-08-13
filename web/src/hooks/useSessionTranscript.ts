import { useCallback, useEffect, useRef, useState } from 'react';
import { fetchEventSource } from '@microsoft/fetch-event-source';
import type { TranscriptEntry, TranscriptSnapshot } from '../types';
import { adminApi } from '../lib/api';
import { notifyUnauthorized, readAdminToken } from '../lib/auth';
import {
  applyTranscriptEntries,
  parseStreamProblem,
  parseTranscriptStreamEvent,
  streamLatencyMs,
} from '../lib/transcript';

export type TranscriptStreamStatus =
  | 'loading'
  | 'connecting'
  | 'live'
  | 'reconnecting'
  | 'offline'
  | 'recovery_needed';

export interface TranscriptRecoveryNotice {
  kind: 'cursor_reset' | 'history_gap' | 'stream_lagged' | 'snapshot_failed';
  message: string;
  oldestSequence?: number;
  nextSequence?: number;
}

interface UseSessionTranscriptResult {
  entries: TranscriptEntry[];
  status: TranscriptStreamStatus;
  latencyMs?: number;
  recovery?: TranscriptRecoveryNotice;
  reload: () => void;
}

const cursorStorageKey = (sessionId: string) =>
  `openab.session-transcript.last-event-id.${sessionId}`;

class AuthenticationStreamError extends Error {}
class RecoveryRequiredError extends Error {}

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}

function snapshotTailCursor(snapshot: TranscriptSnapshot): number | undefined {
  return snapshot.next_sequence > 1 ? snapshot.next_sequence - 1 : undefined;
}

function streamCursorFromSnapshot(snapshot: TranscriptSnapshot): string | undefined {
  if (
    !snapshot.stream_generation ||
    snapshot.stream_next_sequence === undefined ||
    snapshot.stream_next_sequence < 1
  ) {
    return undefined;
  }
  return `${snapshot.stream_generation}:${snapshot.stream_next_sequence - 1}`;
}

function recoveryFromStreamMessage(
  event: string,
  data: string,
): TranscriptRecoveryNotice | undefined {
  const problem = parseStreamProblem(data);
  if (event === 'cursor_reset') {
    return {
      kind: 'cursor_reset',
      message: '服务事件游标已重置。为避免乱序或遗漏，请重新拉取会话快照。',
    };
  }
  if (problem === 'event history unavailable') {
    try {
      const payload = JSON.parse(data) as {
        oldest_sequence?: unknown;
        next_sequence?: unknown;
      };
      return {
        kind: 'history_gap',
        message: '检测到事件历史缺口。请重新拉取 transcript 快照，不会静默丢弃记录。',
        oldestSequence:
          typeof payload.oldest_sequence === 'number'
            ? payload.oldest_sequence
            : undefined,
        nextSequence:
          typeof payload.next_sequence === 'number'
            ? payload.next_sequence
            : undefined,
      };
    } catch {
      return {
        kind: 'history_gap',
        message: '检测到事件历史缺口。请重新拉取 transcript 快照，不会静默丢弃记录。',
      };
    }
  }
  if (problem === 'event stream lagged') {
    return {
      kind: 'stream_lagged',
      message: '实时流消费落后于服务端。请重新拉取 transcript 快照以恢复完整记录。',
    };
  }
  return undefined;
}

/**
 * Restores a read-only transcript in a fixed order: full snapshot, a local
 * transcript tail request, then SSE. SSE revisions use stable entry IDs and
 * are therefore upserted instead of appended.
 */
export function useSessionTranscript(
  sessionId: string,
): UseSessionTranscriptResult {
  const [entries, setEntries] = useState<TranscriptEntry[]>([]);
  const [status, setStatus] = useState<TranscriptStreamStatus>('offline');
  const [latencyMs, setLatencyMs] = useState<number>();
  const [recovery, setRecovery] = useState<TranscriptRecoveryNotice>();
  const [reloadVersion, setReloadVersion] = useState(0);
  const entriesRef = useRef<TranscriptEntry[]>([]);

  const reload = useCallback(() => {
    setReloadVersion((version) => version + 1);
  }, []);

  useEffect(() => {
    if (!sessionId) {
      entriesRef.current = [];
      setEntries([]);
      setStatus('offline');
      return;
    }

    let stopped = false;
    let retryCount = 0;
    let streamCursor = sessionStorage.getItem(cursorStorageKey(sessionId)) || '';
    const controller = new AbortController();
    const setVisibleEntries = (next: TranscriptEntry[]) => {
      entriesRef.current = next;
      setEntries(next);
    };

    const restoreSnapshotAndTail = async (): Promise<boolean> => {
      setStatus('loading');
      setRecovery(undefined);
      try {
        const snapshot = await adminApi.transcript(sessionId);
        if (stopped) return false;
        setVisibleEntries(snapshot.entries);
        streamCursor = streamCursorFromSnapshot(snapshot) || streamCursor;

        const after = snapshotTailCursor(snapshot);
        if (after === undefined) return true;
        const tail = await adminApi.transcript(sessionId, after);
        if (stopped) return false;
        if (tail.overflowed) {
          setRecovery({
            kind: 'history_gap',
            message: '恢复期间发现 transcript 历史缺口。请重新拉取完整快照。',
            oldestSequence: tail.oldest_sequence,
            nextSequence: tail.next_sequence,
          });
          setStatus('recovery_needed');
          return false;
        }
        if (tail.entries.length) {
          setVisibleEntries(applyTranscriptEntries(entriesRef.current, tail.entries));
        }
        streamCursor = streamCursorFromSnapshot(tail) || streamCursor;
        return true;
      } catch {
        if (!stopped) {
          setRecovery({
            kind: 'snapshot_failed',
            message: '无法获取 transcript 快照，请检查连接后重试。',
          });
          setStatus('recovery_needed');
        }
        return false;
      }
    };

    const connect = async () => {
      const restored = await restoreSnapshotAndTail();
      if (!restored || stopped) return;

      while (!stopped) {
        const token = readAdminToken();
        if (!token) {
          setStatus('offline');
          return;
        }

        setStatus(retryCount ? 'reconnecting' : 'connecting');
        try {
          await fetchEventSource('/api/v1/sessions/events', {
            signal: controller.signal,
            openWhenHidden: true,
            headers: {
              Authorization: 'Bearer ' + token,
              ...(streamCursor ? { 'Last-Event-ID': streamCursor } : {}),
            },
            async onopen(response) {
              if (response.status === 401) {
                notifyUnauthorized();
                throw new AuthenticationStreamError('unauthorized');
              }
              if (!response.ok) {
                throw new Error('SSE connection failed: ' + response.status);
              }
              retryCount = 0;
              setStatus('live');
            },
            onmessage(message) {
              const recoveryNotice = recoveryFromStreamMessage(
                message.event,
                message.data,
              );
              if (recoveryNotice) {
                sessionStorage.removeItem(cursorStorageKey(sessionId));
                setRecovery(recoveryNotice);
                setStatus('recovery_needed');
                throw new RecoveryRequiredError(recoveryNotice.message);
              }

              if (message.id) {
                streamCursor = message.id;
                sessionStorage.setItem(cursorStorageKey(sessionId), message.id);
              }
              if (message.event !== 'transcript') return;

              const event = parseTranscriptStreamEvent(message.data);
              if (!event || event.session_id !== sessionId) return;
              setVisibleEntries(
                applyTranscriptEntries(entriesRef.current, [event.entry]),
              );
              setLatencyMs(streamLatencyMs(event.entry.timestamp));
            },
            onclose() {
              throw new Error('SSE connection closed');
            },
            onerror(error) {
              throw error;
            },
          });
        } catch (error) {
          if (stopped || controller.signal.aborted) return;
          if (error instanceof AuthenticationStreamError) {
            setStatus('offline');
            return;
          }
          if (error instanceof RecoveryRequiredError) return;
          retryCount += 1;
          setStatus('reconnecting');
          await wait(Math.min(1_000 * 2 ** (retryCount - 1), 15_000));
        }
      }
    };

    void connect();
    return () => {
      stopped = true;
      controller.abort();
    };
  }, [reloadVersion, sessionId]);

  return { entries, status, latencyMs, recovery, reload };
}
