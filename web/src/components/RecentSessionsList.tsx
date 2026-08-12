import { EyeOutlined, ReloadOutlined } from '@ant-design/icons';
import { Button, Empty, List, Skeleton, Space, Typography } from 'antd';
import { useNavigate } from 'react-router-dom';
import { SessionSnapshot } from '../types';
import { formatRelativeTime } from '../lib/format';
import { sortSessions } from '../lib/session';
import { EntityMark } from './EntityMark';
import { StatusTag } from './StatusTag';

interface RecentSessionsListProps {
  sessions: SessionSnapshot[];
  loading?: boolean;
  limit?: number;
  title?: string;
  onReload?: () => void;
}

export function RecentSessionsList({
  sessions,
  loading = false,
  limit = 8,
  title = '最近会话',
  onReload,
}: RecentSessionsListProps) {
  const navigate = useNavigate();
  const items = sortSessions(sessions).slice(0, limit);

  return (
    <section className="recent-sessions-list" aria-label={title}>
      <div className="recent-sessions-list-header">
        <Space size={10} align="center">
          <Typography.Title level={4}>{title}</Typography.Title>
          <span className="table-count">共 {sessions.length} 条</span>
        </Space>
        {onReload ? (
          <Button
            type="text"
            icon={<ReloadOutlined />}
            onClick={onReload}
            aria-label="刷新最近会话"
          />
        ) : null}
      </div>
      {loading ? (
        <div className="recent-sessions-loading">
          <Skeleton active paragraph={{ rows: 4 }} />
        </div>
      ) : items.length ? (
        <List
          dataSource={items}
          rowKey="session_id"
          renderItem={(session) => (
            <List.Item
              actions={[
                <Button
                  key="view"
                  type="text"
                  icon={<EyeOutlined />}
                  aria-label="查看会话详情"
                  onClick={() =>
                    navigate('/sessions/' + encodeURIComponent(session.session_id))
                  }
                />,
              ]}
            >
              <List.Item.Meta
                avatar={<EntityMark name={session.agent} />}
                title={
                  <Space size={8}>
                    <Typography.Text strong>{session.session_id}</Typography.Text>
                    <StatusTag status={session.status} />
                  </Space>
                }
                description={
                  <Typography.Text type="secondary" ellipsis>
                    {session.agent || '-'} · {session.profile_name || session.profile_id || '默认 Profile'} ·{' '}
                    {formatRelativeTime(session.updated_at)}
                  </Typography.Text>
                }
              />
            </List.Item>
          )}
        />
      ) : (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description="暂无会话"
        />
      )}
    </section>
  );
}
