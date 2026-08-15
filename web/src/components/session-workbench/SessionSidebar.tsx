import { useMemo, useState } from 'react';
import {
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
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
import {
  filterSessions,
  matchesSessionKeyword,
  sessionListSubtitle,
  sessionListTitle,
  sessionProjectGroup,
} from '../../lib/session';
import {
  agentDisplayName,
  formatRelativeTime,
  sessionStatusDisplay,
  sessionStatusOptions,
} from '../../lib/format';
import { AgentProfile, SessionFilters, SessionSnapshot } from '../../types';

interface SessionSidebarProps {
  sessions: SessionSnapshot[];
  profiles?: AgentProfile[];
  loading?: boolean;
  creatingSession?: boolean;
  activeSessionId?: string;
  activeSessionTitle?: string;
  onSelect: (sessionId: string) => void;
  onReload?: () => void;
  onNewAgent?: () => void;
  onCreateSession?: (profile: AgentProfile) => void;
}

const statusOptions = [
  { label: '全部状态', value: '' },
  ...sessionStatusOptions.filter((option) => option.value !== 'unknown'),
];

export function SessionSidebar({
  sessions,
  profiles = [],
  loading,
  creatingSession = false,
  activeSessionId,
  activeSessionTitle,
  onSelect,
  onReload,
  onNewAgent,
  onCreateSession,
}: SessionSidebarProps) {
  const [search, setSearch] = useState('');
  const [filters, setFilters] = useState<SessionFilters>({});
  const [agentScope, setAgentScope] = useState('');

  const platformOptions = useMemo(() => {
    const platforms = [
      ...new Set(sessions.map((session) => session.source?.platform).filter(Boolean)),
    ];
    return [
      { label: '全部平台', value: '' },
      ...platforms.map((value) => ({ label: value, value })),
    ];
  }, [sessions]);

  const agentOptions = useMemo(() => {
    const names = Array.from(
      new Set(sessions.map((session) => agentDisplayName(session.agent))),
    ).sort();
    return [
      { label: '全部 Agent', value: '' },
      ...names.map((name) => ({ label: name, value: name })),
    ];
  }, [sessions]);

  const selectedProfile = useMemo(() => {
    if (agentScope.startsWith('profile:')) {
      const profileId = agentScope.slice('profile:'.length);
      return profiles.find((profile) => profile.id === profileId && profile.enabled);
    }
    return profiles.find((profile) => profile.enabled);
  }, [agentScope, profiles]);

  const filteredSessions = useMemo(() => {
    return filterSessions(sessions, filters).filter((session) => {
      if (agentScope && agentDisplayName(session.agent) !== agentScope) {
        return false;
      }
      return matchesSessionKeyword(session, search);
    });
  }, [agentScope, filters, search, sessions]);

  // ZER-715 P0-2: group by project (working directory) instead of agent type,
  // so the tree reads like a task list rather than a list of runtimes.
  const groupedSessions = useMemo(() => {
    const groups = new Map<string, SessionSnapshot[]>();
    for (const session of filteredSessions) {
      const key = sessionProjectGroup(session);
      const list = groups.get(key) || [];
      list.push(session);
      groups.set(key, list);
    }
    return [...groups.entries()];
  }, [filteredSessions]);

  const newChatTitle = selectedProfile
    ? `使用 ${selectedProfile.name || selectedProfile.id} 新建会话`
    : '请先准备一个启用的 Profile';

  return (
    <aside className="workbench-panel workbench-sidebar" aria-label="会话列表">
      <div className="workbench-panel-header">
        <div>
          <Typography.Text strong>会话历史</Typography.Text>
          <Typography.Text type="secondary" className="workbench-panel-caption">
            按项目分组 · {filteredSessions.length} 条
          </Typography.Text>
        </div>
        <Space size={2} wrap>
          {onNewAgent ? (
            <Tooltip title="New Agent">
              <Button
                type="text"
                size="small"
                icon={<UserAddOutlined />}
                aria-label="New Agent"
                onClick={onNewAgent}
              />
            </Tooltip>
          ) : null}
          <Tooltip title={newChatTitle}>
            <Button
              type="text"
              size="small"
              icon={<PlusOutlined />}
              aria-label="New chat"
              disabled={!selectedProfile || !onCreateSession}
              loading={creatingSession}
              onClick={() => selectedProfile && onCreateSession?.(selectedProfile)}
            />
          </Tooltip>
          {onReload ? (
            <Button
              type="text"
              size="small"
              icon={<ReloadOutlined />}
              aria-label="刷新会话列表"
              onClick={onReload}
            />
          ) : null}
        </Space>
      </div>

      <div className="workbench-sidebar-filters">
        <Select
          value={agentScope}
          options={agentOptions}
          onChange={setAgentScope}
          className="session-agent-filter"
          aria-label="按 Agent 筛选会话"
        />
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
        ) : groupedSessions.length ? (
          <nav className="session-tree" aria-label="按项目分组的会话">
            {groupedSessions.map(([group, items]) => (
              <section key={group} className="session-tree-group">
                <h3 className="session-tree-group-title">{group}</h3>
                <ul className="session-tree-list">
                  {items.map((session) => {
                    const active = session.session_id === activeSessionId;
                    const status =
                      sessionStatusDisplay[session.status] ||
                      sessionStatusDisplay.unknown;
                    return (
                      <li key={session.session_id}>
                        <button
                          type="button"
                          className={
                            'session-tree-item' +
                            (active ? ' is-active' : '')
                          }
                          onClick={() => onSelect(session.session_id)}
                        >
                          <span
                            className={
                              'session-tree-status is-' + session.status
                            }
                            title={status.label}
                            aria-label={status.label}
                          />
                          <span className="session-tree-copy">
                            <span className="session-tree-title">
                              {sessionListTitle(
                                session,
                                active ? activeSessionTitle : undefined,
                              )}
                            </span>
                            <span className="session-tree-subtitle">
                              {sessionListSubtitle(session)}
                              <span className="session-tree-time">
                                {formatRelativeTime(session.updated_at)}
                              </span>
                            </span>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
          </nav>
        ) : (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              search || filters.status || filters.platform || agentScope
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
