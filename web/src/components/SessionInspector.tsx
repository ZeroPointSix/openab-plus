import {
  Alert,
  Descriptions,
  Empty,
  Space,
  Tabs,
  Timeline,
  Typography,
} from 'antd';
import {
  AlertOutlined,
  ClockCircleOutlined,
  ProfileOutlined,
} from '@ant-design/icons';
import { SessionSnapshot, SessionTimelineItem } from '../types';
import { eventLabel, formatDateTime } from '../lib/format';
import { StatusTag } from './StatusTag';

interface SessionInspectorProps {
  session?: SessionSnapshot;
  timeline: SessionTimelineItem[];
}

export function SessionInspector({ session, timeline }: SessionInspectorProps) {
  if (!session) {
    return (
      <aside className="session-inspector" aria-label="会话检查器">
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无会话" />
      </aside>
    );
  }

  return (
    <aside className="session-inspector" aria-label="会话检查器">
      <div className="session-inspector-heading">
        <div>
          <Typography.Title level={4}>Inspector</Typography.Title>
          <Typography.Text type="secondary">运行上下文与异常信号</Typography.Text>
        </div>
        <StatusTag status={session.status} />
      </div>

      <Tabs
        size="small"
        items={[
          {
            key: 'metadata',
            label: '元数据',
            icon: <ProfileOutlined />,
            children: <MetadataPanel session={session} />,
          },
          {
            key: 'alerts',
            label: '告警',
            icon: <AlertOutlined />,
            children: <AlertPanel session={session} />,
          },
          {
            key: 'timeline',
            label: '事件时间线',
            icon: <ClockCircleOutlined />,
            children: <InspectorTimeline timeline={timeline} />,
          },
        ]}
      />
    </aside>
  );
}

function MetadataPanel({ session }: { session: SessionSnapshot }) {
  return (
    <Descriptions column={1} size="small" colon={false} className="inspector-descriptions">
      <Descriptions.Item label="Agent">{session.agent || '-'}</Descriptions.Item>
      <Descriptions.Item label="平台">{session.source.platform || '-'}</Descriptions.Item>
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
        <Space size={6} wrap>
          <span>{session.profile_name || session.profile_id || '-'}</span>
          {session.profile_status === 'deleted' ? (
            <Typography.Text type="danger">已删除</Typography.Text>
          ) : null}
        </Space>
      </Descriptions.Item>
      <Descriptions.Item label="模型">{session.model || '-'}</Descriptions.Item>
      <Descriptions.Item label="创建时间">
        {formatDateTime(session.created_at)}
      </Descriptions.Item>
      <Descriptions.Item label="更新时间">
        {formatDateTime(session.updated_at)}
      </Descriptions.Item>
      <Descriptions.Item label="会话 ID">
        <Typography.Text copyable code>
          {session.session_id}
        </Typography.Text>
      </Descriptions.Item>
    </Descriptions>
  );
}

function AlertPanel({ session }: { session: SessionSnapshot }) {
  const hasConfigErrors = Boolean(session.profile_config_errors?.length);
  const hasLastError = Boolean(session.last_error);

  if (!hasConfigErrors && !hasLastError) {
    return (
      <Empty
        image={Empty.PRESENTED_IMAGE_SIMPLE}
        description="当前没有配置告警"
      />
    );
  }

  return (
    <div className="inspector-alerts">
      {hasLastError ? (
        <Alert
          type="error"
          showIcon
          message="最近错误"
          description={session.last_error}
        />
      ) : null}
      {hasConfigErrors ? (
        <Alert
          type="warning"
          showIcon
          message="Profile 配置告警"
          description={session.profile_config_errors?.map((error) => (
            <div key={error.config_id}>
              {error.config_id}: {error.error}
            </div>
          ))}
        />
      ) : null}
    </div>
  );
}

function InspectorTimeline({ timeline }: { timeline: SessionTimelineItem[] }) {
  if (!timeline.length) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无事件" />;
  }

  return (
    <Timeline
      className="inspector-timeline"
      items={[...timeline].reverse().map((item) => ({
        color: item.status === 'error' ? 'red' : 'blue',
        children: (
          <div>
            <Typography.Text strong>{eventLabel(item.event)}</Typography.Text>
            <br />
            <Typography.Text type="secondary">
              {formatDateTime(item.at)}
            </Typography.Text>
            {item.error ? (
              <Typography.Paragraph type="danger">{item.error}</Typography.Paragraph>
            ) : null}
          </div>
        ),
      }))}
    />
  );
}
