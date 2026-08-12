import {
  BranchesOutlined,
  LinkOutlined,
  LockOutlined,
} from '@ant-design/icons';
import { Button, Divider, Empty, Space, Timeline, Typography } from 'antd';
import { SessionSnapshot, SessionTimelineItem } from '../types';
import { eventLabel, formatDateTime, statusLabels } from '../lib/format';
import { StreamStatus } from '../hooks/useSessionStream';
import { StatusTag } from './StatusTag';

export const streamStatusLabels: Record<StreamStatus, string> = {
  connecting: '正在连接',
  live: '实时连接',
  reconnecting: '正在重连',
  offline: '离线',
};

interface SessionMainPanelProps {
  session?: SessionSnapshot;
  timeline: SessionTimelineItem[];
  streamStatus: StreamStatus;
}

function timelineColor(status: SessionTimelineItem['status']): string {
  if (status === 'error') return 'red';
  if (status === 'running' || status === 'starting') return 'green';
  return 'blue';
}

export function SessionMainPanel({
  session,
  timeline,
  streamStatus,
}: SessionMainPanelProps) {
  if (!session) {
    return (
      <main className="session-main-panel session-main-panel-empty">
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description="从左侧选择一个会话开始观测"
        />
        <ReadOnlyBar streamStatus={streamStatus} />
      </main>
    );
  }

  const orderedTimeline = [...timeline].reverse();

  return (
    <main className="session-main-panel">
      <section className="session-status-bar" aria-label="会话状态">
        <div className="session-status-identity">
          <span className="panel-icon blue" aria-hidden="true">
            <BranchesOutlined />
          </span>
          <div className="session-status-title">
            <Typography.Title level={3}>{session.session_id}</Typography.Title>
            <Typography.Text type="secondary">
              只读会话观测 · {session.source.platform || '未知来源'}
            </Typography.Text>
          </div>
        </div>
        <StatusTag status={session.status} />
      </section>

      <section className="session-status-fields" aria-label="当前运行状态">
        <StatusField label="Agent" value={session.agent || '-'} />
        <StatusField
          label="Profile"
          value={session.profile_name || session.profile_id || '-'}
        />
        <StatusField label="模型" value={session.model || '-'} />
        <StatusField label="状态" value={statusLabels[session.status] || '未知'} />
        <StatusField label="工作目录" value={session.workdir || '-'} code />
        {session.external_url ? (
          <div className="session-status-field session-link-field">
            <Typography.Text type="secondary">会话链接</Typography.Text>
            <Button
              type="link"
              size="small"
              icon={<LinkOutlined />}
              href={session.external_url}
              target="_blank"
              rel="noreferrer"
            >
              打开来源
            </Button>
          </div>
        ) : (
          <StatusField label="会话链接" value="-" />
        )}
      </section>

      <Divider className="session-main-divider" />

      <section className="session-event-stream" aria-label="状态事件流">
        <div className="session-section-heading">
          <div>
            <Typography.Title level={4}>状态事件流</Typography.Title>
            <Typography.Text type="secondary">
              当前会话最近 {timeline.length} 条状态事件
            </Typography.Text>
          </div>
          <Typography.Text type="secondary">仅观测</Typography.Text>
        </div>
        {orderedTimeline.length ? (
          <Timeline
            items={orderedTimeline.map((item) => ({
              color: timelineColor(item.status),
              children: (
                <div className="session-event-entry">
                  <div className="session-event-entry-title">
                    <Typography.Text strong>{eventLabel(item.event)}</Typography.Text>
                    <StatusTag status={item.status} />
                  </div>
                  <Typography.Text type="secondary">
                    {formatDateTime(item.at)}
                  </Typography.Text>
                  {item.error ? (
                    <Typography.Paragraph type="danger">
                      {item.error}
                    </Typography.Paragraph>
                  ) : null}
                </div>
              ),
            }))}
          />
        ) : (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无状态事件" />
        )}
      </section>

      <ReadOnlyBar streamStatus={streamStatus} />
    </main>
  );
}

function StatusField({
  label,
  value,
  code = false,
}: {
  label: string;
  value: string;
  code?: boolean;
}) {
  return (
    <div className="session-status-field">
      <Typography.Text type="secondary">{label}</Typography.Text>
      <Typography.Text className={code ? 'session-status-value-code' : undefined} ellipsis={{ tooltip: value }}>
        {value}
      </Typography.Text>
    </div>
  );
}

function ReadOnlyBar({ streamStatus }: { streamStatus: StreamStatus }) {
  return (
    <footer className="session-read-only-bar">
      <Space size={8}>
        <LockOutlined />
        <Typography.Text strong>只读观测</Typography.Text>
        <Typography.Text type="secondary">
          此工作台不会修改会话、模型或 Agent 配置
        </Typography.Text>
      </Space>
      <span className={'session-stream-state is-' + streamStatus}>
        <span className="stream-dot" aria-hidden="true" />
        {streamStatusLabels[streamStatus]}
      </span>
    </footer>
  );
}
