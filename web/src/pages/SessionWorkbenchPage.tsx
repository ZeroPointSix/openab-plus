import { useEffect, useMemo, useState } from 'react';
import { PageContainer } from '@ant-design/pro-components';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { Drawer, message } from 'antd';
import { NewAgentWizard } from '../components/NewAgentWizard';
import { adminApi, ApiError } from '../lib/api';
import { initialTimeline, sortSessions } from '../lib/session';
import { AgentProfile, SessionSnapshot, SessionTimelineItem } from '../types';
import { useMediaQuery } from '../hooks/useMediaQuery';
import { SessionInspector } from '../components/session-workbench/SessionInspector';
import { SessionMainPanel } from '../components/session-workbench/SessionMainPanel';
import { SessionSidebar } from '../components/session-workbench/SessionSidebar';

const COMMON_AGENT_TYPES = [
  'codex',
  'claude',
  'gemini',
  'opencode',
  'kiro',
  'cursor',
  'hermes',
];

export function SessionWorkbenchPage() {
  const params = useParams<{ id?: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const compact = useMediaQuery('(max-width: 1100px)');
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [newAgentOpen, setNewAgentOpen] = useState(false);

  const sessionsQuery = useQuery({
    queryKey: ['sessions'],
    queryFn: adminApi.sessions,
    refetchInterval: 30_000,
  });
  const profilesQuery = useQuery({
    queryKey: ['profiles'],
    queryFn: adminApi.profiles,
  });
  const agentsQuery = useQuery({
    queryKey: ['agents'],
    queryFn: adminApi.agents,
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

  const createSessionMutation = useMutation({
    mutationFn: (profile: AgentProfile) =>
      adminApi.createSession({ profile_id: profile.id }),
    onSuccess: (snapshot) => {
      queryClient.setQueryData<SessionSnapshot[]>(['sessions'], (current = []) => [
        snapshot,
        ...current.filter((session) => session.session_id !== snapshot.session_id),
      ]);
      queryClient.setQueryData(['session', snapshot.session_id], snapshot);
      message.success('新会话已启动');
      setSidebarOpen(false);
      navigate('/sessions/' + encodeURIComponent(snapshot.session_id));
    },
    onError: (error) => {
      message.error(error instanceof ApiError ? error.message : '启动新会话失败');
    },
  });

  const sessions = useMemo(() => {
    const current = sessionsQuery.data || [];
    if (!sessionQuery.data) return current;
    if (current.some((session) => session.session_id === sessionQuery.data?.session_id)) {
      return current;
    }
    return [sessionQuery.data, ...current];
  }, [sessionsQuery.data, sessionQuery.data]);
  const profiles = profilesQuery.data?.profiles || [];
  const agentTypes = useMemo(
    () =>
      Array.from(
        new Set([
          ...COMMON_AGENT_TYPES,
          ...(agentsQuery.data || []).map((agent) => agent.agent_type),
          ...profiles.map((profile) => profile.agent_type),
        ]),
      ).sort(),
    [agentsQuery.data, profiles],
  );

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

  const handleProfileCreated = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['profiles'] }),
      queryClient.invalidateQueries({ queryKey: ['agents'] }),
    ]);
  };

  const timeline = sessionId ? timelineQuery.data || [] : [];

  const sidebar = (
    <SessionSidebar
      sessions={sessions}
      profiles={profiles}
      loading={
        sessionsQuery.isLoading || sessionQuery.isLoading || profilesQuery.isLoading
      }
      activeSessionId={sessionId || undefined}
      creatingSession={createSessionMutation.isPending}
      onSelect={selectSession}
      onReload={() => void sessionsQuery.refetch()}
      onNewAgent={() => setNewAgentOpen(true)}
      onCreateSession={(profile) => createSessionMutation.mutate(profile)}
    />
  );

  const inspector = (
    <SessionInspector
      session={sessionQuery.data}
      timeline={timeline}
      hasSelection={Boolean(sessionId)}
    />
  );

  return (
    <PageContainer
      title="会话工作台"
      subTitle="配置 Agent / Profile · 实时观测会话"
      className="page-container session-workbench-page"
    >
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
          session={sessionQuery.data}
          loading={Boolean(sessionId) && sessionQuery.isLoading}
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
      <NewAgentWizard
        open={newAgentOpen}
        agentTypes={agentTypes}
        onCancel={() => setNewAgentOpen(false)}
        onCreated={handleProfileCreated}
      />
    </PageContainer>
  );
}
