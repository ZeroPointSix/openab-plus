import {
  CheckCircleOutlined,
  ExclamationCircleOutlined,
  FireOutlined,
  RobotOutlined,
  ScheduleOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { Alert, Card, Collapse, Empty, List, Tag, Typography } from 'antd';
import type { ActivityEntry } from '../../types';
import { AcpToolCallCard } from './AcpToolCallCard';
import { TerminalOutput } from './TerminalOutput';

interface SessionActivityFeedProps {
  entries: ActivityEntry[];
  source: 'mock' | 'live';
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

export function SessionActivityFeed({ entries, source }: SessionActivityFeedProps) {
  if (!entries.length) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无活动记录" />;
  }

  return (
    <section className="session-activity-feed" aria-label="Agent 活动流">
      <div className="activity-feed-caption">
        <Typography.Text type="secondary">
          {source === 'mock' ? '固定 mock transcript · 等待 W3 快照接口接入' : '实时 transcript 快照'}
        </Typography.Text>
        <Tag color={source === 'mock' ? 'gold' : 'green'}>{source === 'mock' ? 'Mock' : 'Live'}</Tag>
      </div>
      <div className="activity-feed-items">
        {entries.map((entry) => {
          switch (entry.type) {
            case 'turn':
              return (
                <div className="activity-turn-divider" key={entry.id}>
                  <span>{entry.label}</span>
                </div>
              );
            case 'user':
            case 'assistant':
              return <ActorMessage actor={entry.type} text={entry.text} key={entry.id} />;
            case 'thinking':
              return (
                <Collapse
                  className="activity-thinking"
                  key={entry.id}
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
                <Card className="activity-plan" key={entry.id} size="small">
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
              return <AcpToolCallCard key={entry.id} tool={entry.tool} />;
            case 'terminal':
              return <TerminalOutput key={entry.id} terminal={entry.terminal} />;
            case 'error':
              return (
                <Alert
                  className="activity-error"
                  key={entry.id}
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
        })}
      </div>
    </section>
  );
}
