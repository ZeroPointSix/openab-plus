import { Empty, Typography } from 'antd';
import { eventLabel, formatDateTime } from '../../lib/format';
import { SessionTimelineItem } from '../../types';

interface SessionEventLogProps {
  timeline: SessionTimelineItem[];
}

function lineFor(item: SessionTimelineItem): string {
  const time = formatDateTime(item.at);
  const label = eventLabel(item.event);
  const error = item.error ? ` error=${item.error}` : '';
  return `${time}  ${item.status.padEnd(10)}  ${label}${error}`;
}

export function SessionEventLog({ timeline }: SessionEventLogProps) {
  if (!timeline.length) {
    return (
      <Empty
        image={Empty.PRESENTED_IMAGE_SIMPLE}
        description="暂无执行日志"
        className="timeline-empty-compact"
      />
    );
  }

  const lines = [...timeline].map(lineFor);

  return (
    <section className="session-event-log" aria-label="只读执行日志">
      <Typography.Text type="secondary" className="inspector-events-caption">
        只读日志流，按时间顺序保留最近 60 条生命周期事件
      </Typography.Text>
      <pre className="session-event-log-body">{lines.join('\n')}</pre>
    </section>
  );
}
