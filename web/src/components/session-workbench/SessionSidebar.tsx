import { useMemo, useState } from 'react';
import {
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  UserAddOutlined,
} from '@ant-design/icons';
import { Conversations } from '@ant-design/x';
import {
  Button,
  Empty,
  Input,
  Select,
  Space,
  Spin,
  Tooltip,
  Typography,
} from 'antd';
import { filterSessions, matchesSessionKeyword } from '../../lib/session';
import { formatRelativeTime } from '../../lib/format';
import { AgentProfile, SessionFilters, SessionSnapshot } from '../../types';
import { EntityMark } from '../EntityMark';
import { StatusTag } from '../StatusTag';

interface SessionSidebarProps {
  sessions: SessionSnapshot[];
  profiles?: AgentProfile[];
  loading?: boolean;
  activeSessionId?: string;
  creatingSession?: boolean;
  onSelect: (sessionId: string) => void;
  onReload?: () => void;
  onNewAgent?: () => void;
  onCreateSession?: (profile: AgentProfile) => void;
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
  profiles = [],
  loading,
  activeSessionId,
  creatingSession = false,
  onSelect,
  onReload,
  onNewAgent,
  onCreateSession,
}: SessionSidebarProps) {
  const [search, setSearch] = useState('');
  const [filters, setFilters] = useState<SessionFilters>({});

  const agentScope = filters.profile
    ? 'profile:' + filters.profile
    : filters.agent
      ? 'agent:' + filters.agent
      : '';

  const agentOptions = useMemo(() => {
    const agentTypes = Array.from(
      new Set([
        ...profiles.map((profile) => profile.agent_type),
        ...sessions.map((session) => session.agent),
      ]),
    )
      .filter(Boolean)
      .sort();

    return [
      { label: '全部 Agent', value: '' },
      ...agentTypes.map((agentType) => ({
        label: agentType,
        options: [
          { label: `全部 ${agentType} 会话`, value: `agent:${agentType}` },
          ...profiles
            .filter((profile) => profile.agent_type === agentType)
            .map((profile) => ({
              label: profile.name || profile.id,
              value: `profile:${profile.id}`,
              disabled: !profile.enabled,
            })),
        ],
      })),
    ];
  }, [profiles, sessions]);

  const selectedProfile = useMemo(() => {
    if (agentScope.startsWith('profile:')) {
      const profileId = agentScope.slice('profile:'.length);
      return profiles.find((profile) => profile.id === profileId && profile.enabled);
    }
    if (agentScope.startsWith('agent:')) {
      const agentType = agentScope.slice('agent:'.length);
      const enabledProfiles = profiles.filter(
        (profile) => profile.agent_type === agentType && profile.enabled,
      );
      return enabledProfiles.length === 1 ? enabledProfiles[0] : undefined;
    }
    return undefined;
  }, [agentScope, profiles]);

  const selectAgentScope = (value: string) => {
    setFilters((current) => ({
      ...current,
      agent: value.startsWith('agent:') ? value.slice('agent:'.length) : undefined,
      profile: value.startsWith('profile:')
        ? value.slice('profile:'.length)
        : undefined,
    }));
  };

  const platformOptions = useMemo(() => {
    const platforms = [
      ...new Set(sessions.map((session) => session.source?.platform).filter(Boolean)),
    ];
    return [
      { label: '全部平台', value: '' },
      ...platforms.map((value) => ({ label: value, value })),
    ];
  }, [sessions]);

  const filteredSessions = useMemo(
    () =>
      filterSessions(sessions, filters).filter((session) =>
        matchesSessionKeyword(session, search),
      ),
    [filters, search, sessions],
  );

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

  const newChatTitle = selectedProfile
    ? `使用 ${selectedProfile.name || selectedProfile.id} 新建会话`
    : '请先选择启用的 Profile；若 Agent 只有一个启用 Profile，也可直接新建会话';

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

      <div className="workbench-sidebar-newchat">
        <Space direction="vertical" size={8} style={{ width: '100%' }}>
          <Button block icon={<UserAddOutlined />} onClick={onNewAgent} disabled={!onNewAgent}>
            New Agent
          </Button>
          <Tooltip title={newChatTitle}>
            <span className="workbench-sidebar-newchat-tooltip">
              <Button
                block
                type="primary"
                icon={<PlusOutlined />}
                disabled={!selectedProfile || !onCreateSession}
                loading={creatingSession}
                onClick={() => selectedProfile && onCreateSession?.(selectedProfile)}
              >
                New chat
              </Button>
            </span>
          </Tooltip>
        </Space>
      </div>

      <div className="workbench-sidebar-filters">
        <Input
          allowClear
          prefix={<SearchOutlined />}
          placeholder="搜索会话、Agent、Profile、平台…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <Select
          size="small"
          value={agentScope}
          options={agentOptions}
          onChange={selectAgentScope}
          aria-label="按 Agent 或 Profile 筛选"
          style={{ width: '100%' }}
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
              search || filters.status || filters.platform || filters.agent || filters.profile
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
