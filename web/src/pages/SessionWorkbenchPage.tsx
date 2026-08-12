import { useEffect, useMemo, useState } from 'react';
import { AppstoreOutlined, InfoCircleOutlined } from '@ant-design/icons';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, Drawer, Space, Typography } from 'antd';
import { useNavigate, useOutletContext, useParams } from 'react-router-dom';
import { SessionMainPanel } from '../components/SessionMainPanel';
import { SessionInspector } from '../components/SessionInspector';
import { SessionSidebar } from '../components/SessionSidebar';
import { StreamStatus } from '../hooks/useSessionStream';
import { adminApi } from '../lib/api';
import { initialTimeline } from '../lib/session';
import { SessionSnapshot, SessionTimelineItem } from '../types';

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
    if (current.some((session) => session.session_id === sessionQuery.data?.session_id)) {
      return current;
    }
    return [sessionQuery.data, ...current];
  }, [sessionsQuery.data, sessionQuery.data]);

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

  const sidebar = (
    <SessionSidebar
      sessions={sessions}
      selectedId={sessionId || undefined}
      loading={sessionsQuery.isLoading || sessionQuery.isLoading}
      onSelect={selectSession}
      onReload={() => void sessionsQuery.refetch()}
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
              三栏只读会话工作台
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

      {sessionQuery.isError ? (
        <Typography.Text type="danger" className="session-workbench-error">
          无法加载深链会话，请检查会话 ID 或稍后重试。
        </Typography.Text>
      ) : null}
    </div>
  );
}
