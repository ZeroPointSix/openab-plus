import { useMemo, useState } from 'react';
import { Conversations } from '@ant-design/x';
import type { ConversationsProps } from '@ant-design/x';
import {
  FilterOutlined,
  PlusOutlined,
  ReloadOutlined,
  UserAddOutlined,
} from '@ant-design/icons';
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
import { AgentProfile, SessionSnapshot, SessionStatus } from '../types';
import { filterSessions, matchesSessionKeyword } from '../lib/session';
import { formatRelativeTime, statusLabels } from '../lib/format';
import { EntityMark } from './EntityMark';
import { StatusTag } from './StatusTag';

interface SessionSidebarProps {
  sessions: SessionSnapshot[];
  profiles?: AgentProfile[];
  selectedId?: string;
  loading?: boolean;
  creatingSession?: boolean;
  onSelect: (sessionId: string) => void;
  onReload?: () => void;
  onNewAgent?: () => void;
  onCreateSession?: (profile: AgentProfile) => void;
}

const statusOptions: Array<{ value: SessionStatus | ''; label: string }> = [
  { value: '', label: '全部状态' },
  { value: 'starting', label: statusLabels.starting },
  { value: 'idle', label: statusLabels.idle },
  { value: 'running', label: statusLabels.running },
  { value: 'suspended', label: statusLabels.suspended },
  { value: 'error', label: statusLabels.error },
  { value: 'exited', label: statusLabels.exited },
];

function groupLabel(session: SessionSnapshot): string {
  const agent = session.agent || '未命名 Agent';
  const profile = session.profile_name || session.profile_id || '默认 Profile';
  return agent + ' / ' + profile;
}

function conversationLabel(session: SessionSnapshot) {
  return (
    <span className="session-conversation-label">
      <span className="session-conversation-title" title={session.session_id}>
        {session.session_id}
      </span>
      <span className="session-conversation-meta">
        <StatusTag status={session.status} />
        <span>{formatRelativeTime(session.updated_at)}</span>
      </span>
    </span>
  );
}

export function SessionSidebar({
  sessions,
  profiles = [],
  selectedId,
  loading = false,
  creatingSession = false,
  onSelect,
  onReload,
  onNewAgent,
  onCreateSession,
}: SessionSidebarProps) {
  const [keyword, setKeyword] = useState('');
  const [status, setStatus] = useState<SessionStatus | ''>('');
  const [agentScope, setAgentScope] = useState('');

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

  const visibleSessions = useMemo(() => {
    const filters = {
      status: status || undefined,
      agent: agentScope.startsWith('agent:')
        ? agentScope.slice('agent:'.length)
        : undefined,
      profile: agentScope.startsWith('profile:')
        ? agentScope.slice('profile:'.length)
        : undefined,
    };
    return filterSessions(sessions, filters).filter((session) =>
      matchesSessionKeyword(session, keyword),
    );
  }, [agentScope, keyword, sessions, status]);

  const items = useMemo<NonNullable<ConversationsProps['items']>>(
    () =>
      visibleSessions.map((session) => ({
        key: session.session_id,
        group: groupLabel(session),
        icon: <EntityMark name={session.agent} size={30} />,
        label: conversationLabel(session),
      })),
    [visibleSessions],
  );

  const newChatTitle = selectedProfile
    ? `使用 ${selectedProfile.name || selectedProfile.id} 新建会话`
    : '请先选择一个启用的 Profile；选择 Agent 类型时仅在该类型只有一个启用 Profile 时可直接新建会话';

  return (
    <aside className="session-sidebar" aria-label="会话历史">
      <div className="session-sidebar-header">
        <div>
          <Typography.Title level={4}>会话工作台</Typography.Title>
          <Typography.Text type="secondary">
            {visibleSessions.length} / {sessions.length} 条会话
          </Typography.Text>
        </div>
        <Space size={2} wrap>
          {onNewAgent ? (
            <Tooltip title="New Agent">
              <Button
                type="text"
                icon={<UserAddOutlined />}
                aria-label="New Agent"
                onClick={onNewAgent}
              />
            </Tooltip>
          ) : null}
          <Tooltip title={newChatTitle}>
            <Button
              type="text"
              icon={<PlusOutlined />}
              aria-label="New chat"
              disabled={!selectedProfile || !onCreateSession}
              loading={creatingSession}
              onClick={() => selectedProfile && onCreateSession?.(selectedProfile)}
            />
          </Tooltip>
          {onReload ? (
            <Tooltip title="刷新会话">
              <Button
                type="text"
                icon={<ReloadOutlined />}
                onClick={onReload}
                aria-label="刷新会话"
              />
            </Tooltip>
          ) : null}
        </Space>
      </div>

      <div className="session-sidebar-filters">
        <Select
          value={agentScope}
          options={agentOptions}
          onChange={setAgentScope}
          className="session-agent-filter"
          aria-label="按 Agent 或 Profile 切换会话"
        />
        <Input
          allowClear
          prefix={<FilterOutlined />}
          placeholder="搜索会话、Agent、Profile 或工作目录"
          value={keyword}
          onChange={(event) => setKeyword(event.target.value)}
          aria-label="搜索会话"
        />
        <Select
          value={status}
          options={statusOptions}
          onChange={setStatus}
          className="session-status-filter"
          aria-label="按状态筛选"
        />
      </div>

      <div className="session-sidebar-list">
        {loading && !sessions.length ? (
          <div className="session-sidebar-loading">
            <Spin />
          </div>
        ) : visibleSessions.length ? (
          <Conversations
            items={items}
            activeKey={selectedId}
            groupable={{
              title: (group) => (
                <span className="session-group-title">{group}</span>
              ),
            }}
            onActiveChange={onSelect}
            rootClassName="session-conversations"
            aria-label="会话列表"
          />
        ) : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              keyword || status || agentScope ? '没有匹配的会话' : '暂无会话'
            }
          />
        )}
      </div>
    </aside>
  );
}
