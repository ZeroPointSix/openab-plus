import { useEffect, useMemo, useState } from 'react';
import { AppstoreOutlined, InfoCircleOutlined } from '@ant-design/icons';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, Drawer, Space, Typography, message } from 'antd';
import { useNavigate, useOutletContext, useParams } from 'react-router-dom';
import { NewAgentWizard } from '../components/NewAgentWizard';
import { SessionMainPanel } from '../components/SessionMainPanel';
import { SessionInspector } from '../components/SessionInspector';
import { SessionSidebar } from '../components/SessionSidebar';
import { StreamStatus } from '../hooks/useSessionStream';
import { adminApi, ApiError } from '../lib/api';
import { initialTimeline } from '../lib/session';
import { AgentProfile, SessionSnapshot, SessionTimelineItem } from '../types';

const COMMON_AGENT_TYPES = [
  'codex',
  'claude',
  'gemini',
  'opencode',
  'kiro',
  'cursor',
  'hermes',
];

export interface AdminLayoutOutletContext {
  streamStatus: StreamStatus;
}

export function SessionWorkbenchPage() {
  const { id } = useParams<{ id: string }>();
  const sessionId = id || '';
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { streamStatus } = useOutletContext<AdminLayoutOutletContext>();
  const [mobilePanel, setMobilePanel] = useState<'sidebar' | 'inspector' | null>(
    null,
  );
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
      setMobilePanel(null);
      navigate('/sessions/' + encodeURIComponent(snapshot.session_id));
    },
    onError: (error) => {
      message.error(
        error instanceof ApiError ? error.message : '启动新会话失败',
      );
    },
  });

  useEffect(() => {
    if (!sessionQuery.data || !sessionId) return;
    queryClient.setQueryData<SessionTimelineItem[]>(
      ['sessionTimeline', sessionId],
      (current = []) =>
        current.length ? current : initialTimeline(sessionQuery.data),
    );
  }, [queryClient, sessionId, sessionQuery.data]);

  const sessions = useMemo(() => {
    const current = sessionsQuery.data || [];
    if (!sessionQuery.data) return current;
    if (
      current.some(
        (session) => session.session_id === sessionQuery.data?.session_id,
      )
    ) {
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

  const selectedSession = useMemo(
    () =>
      sessions.find((session) => session.session_id === sessionId) ||
      sessionQuery.data,
    [sessionId, sessionQuery.data, sessions],
  );

  const selectSession = (nextId: string) => {
    setMobilePanel(null);
    navigate('/sessions/' + encodeURIComponent(nextId));
  };

  const handleProfileCreated = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['profiles'] }),
      queryClient.invalidateQueries({ queryKey: ['agents'] }),
    ]);
  };

  const sidebar = (
    <SessionSidebar
      sessions={sessions}
      profiles={profiles}
      selectedId={sessionId || undefined}
      loading={
        sessionsQuery.isLoading || sessionQuery.isLoading || profilesQuery.isLoading
      }
      creatingSession={createSessionMutation.isPending}
      onSelect={selectSession}
      onReload={() => void sessionsQuery.refetch()}
      onNewAgent={() => setNewAgentOpen(true)}
      onCreateSession={(profile) => createSessionMutation.mutate(profile)}
    />
  );
  const inspector = (
    <SessionInspector
      session={selectedSession}
      timeline={timelineQuery.data || []}
    />
  );

  return (
    <div className="session-workbench-page">
      <header className="session-workbench-header">
        <div className="session-workbench-title">
          <span className="panel-icon blue" aria-hidden="true">
            <AppstoreOutlined />
          </span>
          <div>
            <Typography.Title level={2}>Sessions</Typography.Title>
            <Typography.Text type="secondary">
              按 Agent/Profile 切换的会话工作台
            </Typography.Text>
          </div>
        </div>
        <Space className="session-mobile-actions" size={8}>
          <Button onClick={() => setMobilePanel('sidebar')}>会话列表</Button>
          <Button
            icon={<InfoCircleOutlined />}
            onClick={() => setMobilePanel('inspector')}
          >
            Inspector
          </Button>
        </Space>
      </header>

      <div className="session-workbench-grid">
        <div className="session-workbench-sidebar">{sidebar}</div>
        <SessionMainPanel
          session={selectedSession}
          timeline={timelineQuery.data || []}
          streamStatus={streamStatus}
        />
        <div className="session-workbench-inspector">{inspector}</div>
      </div>

      <Drawer
        title="会话列表"
        placement="left"
        open={mobilePanel === 'sidebar'}
        onClose={() => setMobilePanel(null)}
        width={Math.min(360, window.innerWidth - 24)}
        className="session-mobile-drawer"
        destroyOnClose={false}
      >
        {sidebar}
      </Drawer>
      <Drawer
        title="Inspector"
        placement="right"
        open={mobilePanel === 'inspector'}
        onClose={() => setMobilePanel(null)}
        width={Math.min(380, window.innerWidth - 24)}
        className="session-mobile-drawer"
        destroyOnClose={false}
      >
        {inspector}
      </Drawer>

      <NewAgentWizard
        open={newAgentOpen}
        agentTypes={agentTypes}
        onCancel={() => setNewAgentOpen(false)}
        onCreated={handleProfileCreated}
      />

      {sessionQuery.isError ? (
        <Typography.Text type="danger" className="session-workbench-error">
          无法加载深链会话，请检查会话 ID 或稍后重试。
        </Typography.Text>
      ) : null}
    </div>
  );
}
