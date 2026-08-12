import { useEffect, useRef, useState } from 'react';
import { fetchEventSource } from '@microsoft/fetch-event-source';
import { useQueryClient } from '@tanstack/react-query';
import {
  SessionSnapshot,
  SessionTimelineItem,
} from '../types';
import { notifyUnauthorized, readAdminToken } from '../lib/auth';
import {
  applySessionEvent,
  mergeTimelineItem,
  parseSessionEventPayload,
  timelineItemFromEvent,
} from '../lib/session';

export type StreamStatus = 'connecting' | 'live' | 'reconnecting' | 'offline';

class AuthenticationStreamError extends Error {}

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}

export function useSessionStream(enabled: boolean): StreamStatus {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<StreamStatus>('offline');
  // A reload starts without a cursor so the one app-wide SSE connection replays
  // session history before it switches to live delivery. The ref still preserves
  // the cursor for reconnects within the current page lifetime.
  const lastEventId = useRef('');

  useEffect(() => {
    if (!enabled) {
      setStatus('offline');
      return;
    }

    let stopped = false;
    let retryCount = 0;
    const controller = new AbortController();

    const connect = async () => {
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
              ...(lastEventId.current
                ? { 'Last-Event-ID': lastEventId.current }
                : {}),
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
              if (message.id) {
                lastEventId.current = message.id;
              }

              const event = parseSessionEventPayload(message.data);
              if (!event) {
                queryClient.setQueriesData<SessionTimelineItem[]>(
                  { queryKey: ['sessionTimeline'] },
                  [],
                );
                void queryClient.invalidateQueries({ queryKey: ['sessions'] });
                void queryClient.invalidateQueries({ queryKey: ['session'] });
                return;
              }

              if (event.sequence && !message.id) {
                lastEventId.current = String(event.sequence);
              }
              queryClient.setQueryData<SessionSnapshot[]>(
                ['sessions'],
                (current) => applySessionEvent(current, event),
              );
              if (event.snapshot) {
                queryClient.setQueryData(
                  ['session', event.snapshot.session_id],
                  event.snapshot,
                );
                const timelineItem = timelineItemFromEvent(
                  event,
                  message.id,
                );
                if (timelineItem) {
                  queryClient.setQueryData<SessionTimelineItem[]>(
                    ['sessionTimeline', event.snapshot.session_id],
                    (current) => mergeTimelineItem(current, timelineItem),
                  );
                }
              }
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
  }, [enabled, queryClient]);

  return status;
}
