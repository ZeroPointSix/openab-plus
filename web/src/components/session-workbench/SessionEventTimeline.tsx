import {
  ClockCircleOutlined,
  ExclamationCircleFilled,
} from '@ant-design/icons';
import { Empty, Timeline, Typography } from 'antd';
import { eventLabel, formatDateTime } from '../../lib/format';
import { sessionStatusDisplay } from '../../lib/sessionStatus';
import { SessionTimelineItem } from '../../types';
import { StatusTag } from '../StatusTag';

interface SessionEventTimelineProps {
  timeline: SessionTimelineItem[];
  compact?: boolean;
}

export function SessionEventTimeline({
  timeline,
  compact = false,
}: SessionEventTimelineProps) {
  if (!timeline.length) {
    return (
      <Empty
        image={Empty.PRESENTED_IMAGE_SIMPLE}
        description="暂无事件"
        className={compact ? 'timeline-empty-compact' : undefined}
      />
    );
  }

  return (
    <Timeline
      className={compact ? 'session-timeline-compact' : undefined}
      items={[...timeline].reverse().map((item) => ({
        color: sessionStatusDisplay(item.status).timelineColor,
        dot:
          item.status === 'error' ? (
            <ExclamationCircleFilled />
          ) : (
            <ClockCircleOutlined />
          ),
        children: (
          <div className="timeline-entry">
            <div className="timeline-title">
              <Typography.Text strong={!compact}>
                {eventLabel(item.event)}
              </Typography.Text>
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
  );
}
