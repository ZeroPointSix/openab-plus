import { useEffect, useRef, useState } from 'react';
import { fetchEventSource } from '@microsoft/fetch-event-source';
import { useQueryClient } from '@tanstack/react-query';
import {
  SessionSnapshot,
  SessionTimelineItem,
} from '../types';
import {
  LAST_EVENT_ID_KEY,
  notifyUnauthorized,
  readAdminToken,
} from '../lib/auth';
import {
  applySessionEvent,
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
  const lastEventId = useRef(sessionStorage.getItem(LAST_EVENT_ID_KEY) || '');

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
                sessionStorage.setItem(LAST_EVENT_ID_KEY, message.id);
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
                sessionStorage.setItem(
                  LAST_EVENT_ID_KEY,
                  lastEventId.current,
                );
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
                    (current = []) => {
                      if (
                        current.some((item) => item.id === timelineItem.id)
                      ) {
                        return current;
                      }
                      return [...current, timelineItem].slice(-60);
                    },
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
