import {
  CheckCircleOutlined,
  ExclamationCircleOutlined,
  FireOutlined,
  RobotOutlined,
  ScheduleOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { Alert, Card, Collapse, Empty, List, Typography } from 'antd';
import { Virtuoso } from 'react-virtuoso';
import type { ActivityEntry } from '../../types';
import {
  transcriptStatusLabel,
  type TranscriptStatusKey,
} from '../../lib/format';
import { AcpToolCallCard } from './AcpToolCallCard';
import { TerminalOutput } from './TerminalOutput';

interface SessionActivityFeedProps {
  entries: ActivityEntry[];
  streamStatus: TranscriptStatusKey;
}

function ActorMessage({
  actor,
  text,
}: {
  actor: 'user' | 'assistant';
  text: string;
}) {
  const isUser = actor === 'user';
  return (
    <article className={`activity-message-row ${actor}`}>
      <span className="activity-actor-icon" aria-hidden="true">
        {isUser ? <UserOutlined /> : <RobotOutlined />}
      </span>
      <div className="activity-message-content">
        <Typography.Text className="activity-message-label" strong>
          {isUser ? '用户' : 'Agent'}
        </Typography.Text>
        <Typography.Paragraph>{text}</Typography.Paragraph>
      </div>
    </article>
  );
}

function ActivityFeedItem({ entry }: { entry: ActivityEntry }) {
  switch (entry.type) {
    case 'turn':
      return (
        <div className="activity-turn-divider">
          <span>{entry.label}</span>
        </div>
      );
    case 'user':
    case 'assistant':
      return <ActorMessage actor={entry.type} text={entry.text} />;
    case 'thinking':
      return (
        <Collapse
          className="activity-thinking"
          items={[
            {
              key: 'thinking',
              label: (
                <span>
                  <FireOutlined /> 思考过程
                </span>
              ),
              children: <Typography.Paragraph>{entry.text}</Typography.Paragraph>,
            },
          ]}
        />
      );
    case 'plan':
      return (
        <Card className="activity-plan" size="small">
          <div className="activity-plan-heading">
            <ScheduleOutlined />
            <Typography.Text strong>{entry.title}</Typography.Text>
          </div>
          <List
            size="small"
            split={false}
            dataSource={entry.items}
            renderItem={(item) => (
              <List.Item>
                <span className={item.done ? 'activity-plan-done' : undefined}>
                  <CheckCircleOutlined /> {item.text}
                </span>
              </List.Item>
            )}
          />
        </Card>
      );
    case 'tool':
      return <AcpToolCallCard tool={entry.tool} />;
    case 'terminal':
      return <TerminalOutput terminal={entry.terminal} />;
    case 'error':
      return (
        <Alert
          className="activity-error"
          type="error"
          showIcon
          icon={<ExclamationCircleOutlined />}
          message="活动流错误"
          description={entry.message}
        />
      );
    default:
      return null;
  }
}

export function SessionActivityFeed({
  entries,
  streamStatus,
}: SessionActivityFeedProps) {
  if (!entries.length) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无活动记录" />;
  }

  const statusClass =
    streamStatus === 'recovery_needed' ? 'offline' : streamStatus;

  return (
    <section className="session-activity-feed" aria-label="Agent 活动流">
      <div className="activity-feed-caption">
        <Typography.Text type="secondary">活动流</Typography.Text>
        <span className={'workbench-stream-status is-' + statusClass}>
          <span className="stream-dot" aria-hidden="true" />
          <span className="stream-label">{transcriptStatusLabel(streamStatus)}</span>
        </span>
      </div>
      <div className="activity-feed-items">
        <Virtuoso
          className="activity-feed-virtual-list"
          data={entries}
          computeItemKey={(_, entry) => entry.id}
          itemContent={(_, entry) => (
            <div className="activity-feed-row">
              <ActivityFeedItem entry={entry} />
            </div>
          )}
        />
      </div>
    </section>
  );
}
