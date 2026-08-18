import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ProfileOutlined, UnorderedListOutlined } from '@ant-design/icons';
import { Button, Drawer, Tooltip, message } from 'antd';
import { useNavigate, useParams } from 'react-router-dom';
import { NewAgentWizard } from '../components/NewAgentWizard';
import { NewSessionDrawer } from '../components/NewSessionDrawer';
import { SessionInspector } from '../components/session-workbench/SessionInspector';
import { SessionMainPanel } from '../components/session-workbench/SessionMainPanel';
import { SessionSidebar } from '../components/session-workbench/SessionSidebar';
import { useMediaQuery } from '../hooks/useMediaQuery';
import { useSessionTranscript } from '../hooks/useSessionTranscript';
import { adminApi, ApiError } from '../lib/api';
import { sortSessions, titleFromTranscript } from '../lib/session';
import { CreateSessionRequest, SessionSnapshot } from '../types';

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
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(false);
  const [newAgentOpen, setNewAgentOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);

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

  // ZER-715 P1-5: the workbench used to keep a client-only ['sessionTimeline']
  // query that never fetched anything and was seeded with one or two synthetic
  // items, so the log only ever showed the first couple of runs. The transcript
  // stream is the real server-backed history, and it is owned here so the
  // activity feed and the inspector log share one SSE connection.
  const transcript = useSessionTranscript(sessionId);
  const derivedTitle = useMemo(
    () => titleFromTranscript(transcript.entries),
    [transcript.entries],
  );

  const createSessionMutation = useMutation({
    mutationFn: (request: CreateSessionRequest) =>
      adminApi.createSession(request),
    onSuccess: (snapshot) => {
      queryClient.setQueryData<SessionSnapshot[]>(['sessions'], (current = []) => [
        snapshot,
        ...current.filter((session) => session.session_id !== snapshot.session_id),
      ]);
      queryClient.setQueryData(['session', snapshot.session_id], snapshot);
      message.success('新会话已启动');
      setNewSessionOpen(false);
      setSidebarOpen(false);
      navigate('/sessions/' + encodeURIComponent(snapshot.session_id));
    },
    onError: (error) => {
      message.error(
        error instanceof ApiError ? error.message : '启动新会话失败',
      );
    },
  });

  const sessions = sessionsQuery.data || [];
  const selectedSession = sessionQuery.data;
  const sidebarSessions =
    selectedSession &&
    !sessions.some((session) => session.session_id === selectedSession.session_id)
      ? [selectedSession, ...sessions]
      : sessions;
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

  const sidebar = (
    <SessionSidebar
      sessions={sidebarSessions}
      profiles={profiles}
      loading={sessionsQuery.isLoading || profilesQuery.isLoading}
      creatingSession={createSessionMutation.isPending}
      activeSessionId={sessionId || undefined}
      activeSessionTitle={derivedTitle}
      onSelect={selectSession}
      onReload={() => void sessionsQuery.refetch()}
      onNewAgent={() => setNewAgentOpen(true)}
      onCreateSession={() => setNewSessionOpen(true)}
    />
  );

  const inspector = (
    <SessionInspector
      session={selectedSession}
      entries={transcript.entries}
      hasSelection={Boolean(sessionId)}
    />
  );

  // ZER-715 P0-1: on desktop both side panels collapse to a narrow rail so the
  // activity feed can use the full width.
  const sidebarRail = (
    <div className="workbench-panel workbench-sidebar workbench-rail">
      <Tooltip title="展开会话列表" placement="right">
        <Button
          type="text"
          size="small"
          icon={<UnorderedListOutlined />}
          aria-label="展开会话列表"
          onClick={() => setSidebarCollapsed(false)}
        />
      </Tooltip>
    </div>
  );

  const inspectorRail = (
    <div className="workbench-panel workbench-inspector workbench-rail">
      <Tooltip title="展开会话详情" placement="left">
        <Button
          type="text"
          size="small"
          icon={<ProfileOutlined />}
          aria-label="展开会话详情"
          onClick={() => setInspectorCollapsed(false)}
        />
      </Tooltip>
    </div>
  );

  const workbenchClassName = [
    'session-workbench',
    !compact && sidebarCollapsed ? 'is-sidebar-collapsed' : '',
    !compact && inspectorCollapsed ? 'is-inspector-collapsed' : '',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <main className="session-workbench-page" aria-label="Agent 会话工作台">
      <div className={workbenchClassName}>
        {compact ? (
          <Drawer
            className="workbench-mobile-drawer"
            placement="left"
            width={320}
            open={sidebarOpen}
            onClose={() => setSidebarOpen(false)}
            title={null}
          >
            {sidebar}
          </Drawer>
        ) : sidebarCollapsed ? (
          sidebarRail
        ) : (
          sidebar
        )}
        <SessionMainPanel
          session={selectedSession}
          transcript={transcript}
          loading={Boolean(sessionId) && sessionQuery.isLoading}
          loadError={
            sessionQuery.isError
              ? '无法加载该深链会话，请确认会话仍存在后重试'
              : undefined
          }
          hasSelection={Boolean(sessionId)}
          onToggleSidebar={
            compact
              ? () => setSidebarOpen(true)
              : () => setSidebarCollapsed((current) => !current)
          }
          onToggleInspector={
            compact
              ? () => setInspectorOpen(true)
              : () => setInspectorCollapsed((current) => !current)
          }
          sidebarToggleLabel={
            compact
              ? '打开会话列表'
              : sidebarCollapsed
                ? '展开会话列表'
                : '折叠会话列表'
          }
          inspectorToggleLabel={
            compact
              ? '打开会话详情'
              : inspectorCollapsed
                ? '展开会话详情'
                : '折叠会话详情'
          }
        />
        {compact ? (
          <Drawer
            className="workbench-mobile-drawer"
            placement="right"
            width={340}
            open={inspectorOpen}
            onClose={() => setInspectorOpen(false)}
            title={null}
          >
            {inspector}
          </Drawer>
        ) : inspectorCollapsed ? (
          inspectorRail
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
      <NewSessionDrawer
        open={newSessionOpen}
        profiles={profiles}
        defaultProfile={profilesQuery.data?.default_profile}
        submitting={createSessionMutation.isPending}
        onOpenChange={setNewSessionOpen}
        onSubmit={(request) => createSessionMutation.mutateAsync(request)}
      />
    </main>
  );
}
