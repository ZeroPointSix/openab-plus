import { useEffect } from 'react';
import { PageContainer } from '@ant-design/pro-components';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { adminApi } from '../lib/api';
import { initialTimeline, sortSessions } from '../lib/session';
import { SessionTimelineItem } from '../types';
import { SessionInspector } from '../components/session-workbench/SessionInspector';
import { SessionMainPanel } from '../components/session-workbench/SessionMainPanel';
import { SessionSidebar } from '../components/session-workbench/SessionSidebar';

export function SessionWorkbenchPage() {
  const params = useParams<{ id?: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
  const queryClient = useQueryClient();

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
  };

  const timeline = sessionId ? timelineQuery.data || [] : [];

  return (
    <PageContainer
      title="会话工作台"
      subTitle="只读观测 Agent 会话 · 不提供发送或控制入口"
      className="page-container session-workbench-page"
    >
      <div className="session-workbench">
        <SessionSidebar
          sessions={sessions}
          loading={sessionsQuery.isLoading}
          activeSessionId={sessionId || undefined}
          onSelect={selectSession}
          onReload={() => void sessionsQuery.refetch()}
        />
        <SessionMainPanel
          session={sessionQuery.data}
          loading={Boolean(sessionId) && sessionQuery.isLoading}
          hasSelection={Boolean(sessionId)}
        />
        <SessionInspector
          session={sessionQuery.data}
          timeline={timeline}
          hasSelection={Boolean(sessionId)}
        />
      </div>
    </PageContainer>
  );
}
