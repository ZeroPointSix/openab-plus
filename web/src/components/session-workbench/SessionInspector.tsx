import {
  AlertOutlined,
  BranchesOutlined,
  ProfileOutlined,
} from '@ant-design/icons';
import { Alert, Descriptions, Empty, Space, Tabs, Typography } from 'antd';
import { agentDisplayName, formatDateTime } from '../../lib/format';
import { SessionSnapshot, SessionTimelineItem } from '../../types';
import { StatusTag } from '../StatusTag';
import { SessionEventTimeline } from './SessionEventTimeline';

interface SessionInspectorProps {
  session?: SessionSnapshot;
  timeline: SessionTimelineItem[];
  hasSelection: boolean;
}

type AlertItem = {
  key: string;
  type: 'error' | 'warning';
  message: string;
  description: string;
};

function isAlertItem(item: AlertItem | null): item is AlertItem {
  return item !== null;
}

export function SessionInspector({
  session,
  timeline,
  hasSelection,
}: SessionInspectorProps) {
  if (!hasSelection) {
    return (
      <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
        <div className="workbench-inspector-empty">
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="选择会话后查看元数据、告警与事件"
          />
        </div>
      </aside>
    );
  }

  if (!session) {
    return (
      <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
        <div className="workbench-inspector-empty">
          <Empty description="会话不可用" />
        </div>
      </aside>
    );
  }

  const alertItems = [
    session.last_error
      ? {
          key: 'last-error',
          type: 'error' as const,
          message: '会话异常',
          description: session.last_error,
        }
      : null,
    ...(session.profile_config_errors || []).map((error) => ({
      key: error.config_id,
      type: 'warning' as const,
      message: 'Profile 配置告警 · ' + error.config_id,
      description: error.error,
    })),
  ].filter(isAlertItem);

  const tabItems = [
    {
      key: 'metadata',
      label: (
        <span className="inspector-tab-label">
          <ProfileOutlined />
          元数据
        </span>
      ),
      children: (
        <div className="inspector-tab-body">
          <div className="inspector-section-heading">
            <StatusTag status={session.status} />
          </div>
          <Descriptions column={1} size="small" colon={false}>
            <Descriptions.Item label="Agent">
              {agentDisplayName(session.agent)}
            </Descriptions.Item>
            <Descriptions.Item label="平台">
              {session.source.platform || '-'}
            </Descriptions.Item>
            <Descriptions.Item label="Thread">
              <Typography.Text copyable code>
                {session.source.thread_id || '-'}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="工作目录">
              <Typography.Text copyable code>
                {session.workdir || '-'}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="Profile">
              <Space size={6}>
                <span>{session.profile_name || session.profile_id || '-'}</span>
                {session.profile_status === 'deleted' ? (
                  <Typography.Text type="danger">已删除</Typography.Text>
                ) : null}
              </Space>
            </Descriptions.Item>
            <Descriptions.Item label="模型">
              {session.model || '--'}
            </Descriptions.Item>
            <Descriptions.Item label="Thinking">
              {session.reasoning_effort || '--'}
            </Descriptions.Item>
            <Descriptions.Item label="元数据来源">
              {session.metadata_source === 'acp'
                ? 'ACP 运行时'
                : session.metadata_source === 'configured'
                  ? '配置值'
                  : '--'}
            </Descriptions.Item>
            <Descriptions.Item label="创建时间">
              {formatDateTime(session.created_at)}
            </Descriptions.Item>
            <Descriptions.Item label="更新时间">
              {formatDateTime(session.updated_at)}
            </Descriptions.Item>
          </Descriptions>
        </div>
      ),
    },
    {
      key: 'alerts',
      label: (
        <span className="inspector-tab-label">
          <AlertOutlined />
          告警
          {alertItems.length ? (
            <span className="inspector-tab-badge">{alertItems.length}</span>
          ) : null}
        </span>
      ),
      children: (
        <div className="inspector-tab-body">
          {alertItems.length ? (
            alertItems.map((item) => (
              <Alert
                key={item.key}
                type={item.type}
                showIcon
                message={item.message}
                description={item.description}
                className="inspector-alert"
              />
            ))
          ) : (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description="暂无告警"
            />
          )}
        </div>
      ),
    },
    {
      key: 'events',
      label: (
        <span className="inspector-tab-label">
          <BranchesOutlined />
          事件
        </span>
      ),
      children: (
        <div className="inspector-tab-body">
          <Typography.Text type="secondary" className="inspector-events-caption">
            实时状态事件，最多保留最近 60 条
          </Typography.Text>
          <SessionEventTimeline timeline={timeline} compact />
        </div>
      ),
    },
  ];

  return (
    <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
      <Tabs
        className="workbench-inspector-tabs"
        items={tabItems}
        size="small"
      />
    </aside>
  );
}
