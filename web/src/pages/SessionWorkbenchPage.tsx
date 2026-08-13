import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { Drawer } from 'antd';
import { adminApi } from '../lib/api';
import { initialTimeline, sortSessions } from '../lib/session';
import { SessionTimelineItem } from '../types';
import { useMediaQuery } from '../hooks/useMediaQuery';
import { SessionInspector } from '../components/session-workbench/SessionInspector';
import { SessionMainPanel } from '../components/session-workbench/SessionMainPanel';
import { SessionSidebar } from '../components/session-workbench/SessionSidebar';

export function SessionWorkbenchPage() {
  const params = useParams<{ id?: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const compact = useMediaQuery('(max-width: 1100px)');
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);

  const sessionsQuery = useQuery({
    queryKey: ['sessions'],
    queryFn: adminApi.sessions,
    refetchInterval: 30_000,
  });

  const sessionQuery = useQuery({
    queryKey: ['session', sessionId],
    queryFn: () => adminApi.session(sessionId),
    enabled: Boolean(sessionId),
  });

  const timelineQuery = useQuery<SessionTimelineItem[]>({
    queryKey: ['sessionTimeline', sessionId],
    queryFn: async () => [],
    enabled: false,
    initialData: [],
  });

  const sessions = sessionsQuery.data || [];
  const selectedSession = sessionQuery.data;
  const sidebarSessions = selectedSession &&
    !sessions.some((session) => session.session_id === selectedSession.session_id)
    ? [selectedSession, ...sessions]
    : sessions;

  useEffect(() => {
    if (sessionId || sessionsQuery.isLoading || sessionsQuery.isFetching) {
      return;
    }

    const [firstSession] = sortSessions(sessions);
    if (firstSession) {
      navigate('/sessions/' + encodeURIComponent(firstSession.session_id), {
        replace: true,
      });
    }
  }, [
    navigate,
    sessionId,
    sessions,
    sessionsQuery.isFetching,
    sessionsQuery.isLoading,
  ]);

  useEffect(() => {
    if (!sessionQuery.data) return;
    queryClient.setQueryData<SessionTimelineItem[]>(
      ['sessionTimeline', sessionId],
      (current = []) =>
        current.length ? current : initialTimeline(sessionQuery.data!),
    );
  }, [queryClient, sessionId, sessionQuery.data]);

  const selectSession = (nextSessionId: string) => {
    navigate('/sessions/' + encodeURIComponent(nextSessionId));
    setSidebarOpen(false);
  };

  const timeline = sessionId ? timelineQuery.data || [] : [];

  const sidebar = (
    <SessionSidebar
      sessions={sidebarSessions}
      loading={sessionsQuery.isLoading}
      activeSessionId={sessionId || undefined}
      onSelect={selectSession}
      onReload={() => void sessionsQuery.refetch()}
    />
  );

  const inspector = (
    <SessionInspector
      session={selectedSession}
      timeline={timeline}
      hasSelection={Boolean(sessionId)}
    />
  );

  return (
    <main className="session-workbench-page" aria-label="只读 Agent 会话工作台">
      <div className="session-workbench">
        {compact ? (
          <Drawer
            className="workbench-drawer"
            placement="left"
            width={320}
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
            title={null}
          >
            {sidebar}
          </Drawer>
        ) : (
          sidebar
        )}
        <SessionMainPanel
          session={selectedSession}
          loading={Boolean(sessionId) && sessionQuery.isLoading}
          loadError={sessionQuery.isError ? '无法加载该深链会话，请确认会话仍存在后重试' : undefined}
          hasSelection={Boolean(sessionId)}
          onOpenSidebar={compact ? () => setSidebarOpen(true) : undefined}
          onOpenInspector={compact ? () => setInspectorOpen(true) : undefined}
        />
        {compact ? (
          <Drawer
            className="workbench-drawer"
            placement="right"
            width={340}
            open={inspectorOpen}
            onClose={() => setInspectorOpen(false)}
            title={null}
          >
            {inspector}
          </Drawer>
        ) : (
          inspector
        )}
      </div>
    </main>
  );
}
