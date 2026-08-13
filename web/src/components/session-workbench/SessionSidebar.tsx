import { useMemo, useState } from 'react';
import {
  ReloadOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import { Conversations } from '@ant-design/x';
import {
  Button,
  Empty,
  Input,
  Select,
  Space,
  Spin,
  Typography,
} from 'antd';
import { filterSessions } from '../../lib/session';
import { formatRelativeTime } from '../../lib/format';
import { SessionFilters, SessionSnapshot } from '../../types';
import { EntityMark } from '../EntityMark';
import { StatusTag } from '../StatusTag';

interface SessionSidebarProps {
  sessions: SessionSnapshot[];
  loading?: boolean;
  activeSessionId?: string;
  onSelect: (sessionId: string) => void;
  onReload?: () => void;
}

const statusOptions = [
  { label: '全部状态', value: '' },
  { label: '启动中', value: 'starting' },
  { label: '空闲', value: 'idle' },
  { label: '运行中', value: 'running' },
  { label: '已暂停', value: 'suspended' },
  { label: '失败', value: 'error' },
  { label: '已退出', value: 'exited' },
];

function sessionTitle(session: SessionSnapshot): string {
  const platform = session.source?.platform || '未知平台';
  const thread = session.source?.thread_id;
  if (thread) {
    return platform + ' · ' + thread.slice(0, 12);
  }
  return platform + ' · ' + session.session_id.slice(0, 12);
}

function agentGroup(session: SessionSnapshot): string {
  const agent = session.agent || '未知 Agent';
  const profile = session.profile_name || session.profile_id;
  return profile ? profile + ' · ' + agent : agent;
}

export function SessionSidebar({
  sessions,
  loading,
  activeSessionId,
  onSelect,
  onReload,
}: SessionSidebarProps) {
  const [search, setSearch] = useState('');
  const [filters, setFilters] = useState<SessionFilters>({});

  const platformOptions = useMemo(() => {
    const platforms = [
      ...new Set(sessions.map((session) => session.source?.platform).filter(Boolean)),
    ];
    return [
      { label: '全部平台', value: '' },
      ...platforms.map((value) => ({ label: value, value })),
    ];
  }, [sessions]);

  const filteredSessions = useMemo(() => {
    const filtered = filterSessions(sessions, filters);
    const query = search.trim().toLowerCase();
    if (!query) return filtered;

    return filtered.filter((session) => {
      const haystack = [
        session.session_id,
        session.agent,
        session.source?.platform,
        session.source?.thread_id,
        session.workdir,
        session.profile_name,
        session.profile_id,
      ]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [filters, search, sessions]);

  const conversationItems = useMemo(
    () =>
      filteredSessions.map((session) => ({
        key: session.session_id,
        group: agentGroup(session),
        timestamp: new Date(session.updated_at || session.created_at).getTime(),
        icon: <EntityMark name={session.agent} size={22} />,
        label: (
          <div className="session-conversation-label">
            <div className="session-conversation-title">
              <Typography.Text ellipsis>{sessionTitle(session)}</Typography.Text>
              <StatusTag status={session.status} />
            </div>
            <Typography.Text type="secondary" className="session-conversation-meta">
              {formatRelativeTime(session.updated_at)}
            </Typography.Text>
          </div>
        ),
      })),
    [filteredSessions],
  );

  return (
    <aside className="workbench-panel workbench-sidebar" aria-label="会话列表">
      <div className="workbench-panel-header">
        <div>
          <Typography.Text strong>Agent 会话</Typography.Text>
          <Typography.Text type="secondary" className="workbench-panel-caption">
            按 Profile / Agent 分组 · {filteredSessions.length} 条
          </Typography.Text>
        </div>
        {onReload ? (
          <Button
            type="text"
            size="small"
            icon={<ReloadOutlined />}
            aria-label="刷新会话列表"
            onClick={onReload}
          />
        ) : null}
      </div>

      <div className="workbench-sidebar-filters">
        <Input
          allowClear
          prefix={<SearchOutlined />}
          placeholder="搜索会话、Agent、平台…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <Space wrap size={8}>
          <Select
            size="small"
            value={filters.status || ''}
            options={statusOptions}
            onChange={(value) =>
              setFilters((current) => ({
                ...current,
                status: value || undefined,
              }))
            }
            popupMatchSelectWidth={false}
          />
          <Select
            size="small"
            value={filters.platform || ''}
            options={platformOptions}
            onChange={(value) =>
              setFilters((current) => ({
                ...current,
                platform: value || undefined,
              }))
            }
            popupMatchSelectWidth={false}
          />
        </Space>
      </div>

      <div className="workbench-sidebar-list">
        {loading ? (
          <div className="workbench-sidebar-loading">
            <Spin />
          </div>
        ) : conversationItems.length ? (
          <Conversations
            className="session-conversations"
            items={conversationItems}
            activeKey={activeSessionId}
            onActiveChange={(key) => {
              if (key) onSelect(key);
            }}
            groupable
          />
        ) : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              search || filters.status || filters.platform
                ? '没有匹配的会话'
                : '暂无会话'
            }
            className="workbench-sidebar-empty"
          />
        )}
      </div>
    </aside>
  );
}
