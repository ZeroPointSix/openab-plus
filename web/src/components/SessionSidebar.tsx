import { useMemo, useState } from 'react';
import { Conversations } from '@ant-design/x';
import type { ConversationsProps } from '@ant-design/x';
import {
  FilterOutlined,
  PlusOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import { Button, Empty, Input, Select, Space, Spin, Tooltip, Typography } from 'antd';
import { SessionSnapshot, SessionStatus } from '../types';
import { filterSessions, matchesSessionKeyword } from '../lib/session';
import { formatRelativeTime, statusLabels } from '../lib/format';
import { EntityMark } from './EntityMark';
import { StatusTag } from './StatusTag';

interface SessionSidebarProps {
  sessions: SessionSnapshot[];
  selectedId?: string;
  loading?: boolean;
  onSelect: (sessionId: string) => void;
  onReload?: () => void;
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
  selectedId,
  loading = false,
  onSelect,
  onReload,
}: SessionSidebarProps) {
  const [keyword, setKeyword] = useState('');
  const [status, setStatus] = useState<SessionStatus | ''>('');

  const visibleSessions = useMemo(() => {
    const statusFiltered = filterSessions(sessions, {
      status: status || undefined,
    });
    return statusFiltered.filter((session) =>
      matchesSessionKeyword(session, keyword),
    );
  }, [keyword, sessions, status]);

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

  return (
    <aside className="session-sidebar" aria-label="会话历史">
      <div className="session-sidebar-header">
        <div>
          <Typography.Title level={4}>会话工作台</Typography.Title>
          <Typography.Text type="secondary">
            {visibleSessions.length} / {sessions.length} 条会话
          </Typography.Text>
        </div>
        <Space size={4}>
          <Tooltip title="New chat（即将开放）">
            <Button
              type="text"
              disabled
              icon={<PlusOutlined />}
              aria-label="New chat（即将开放）"
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
              keyword || status ? '没有匹配的会话' : '暂无会话'
            }
          />
        )}
      </div>
    </aside>
  );
}
