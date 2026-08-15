import { Tag } from 'antd';
import {
  CheckCircleFilled,
  CloseCircleFilled,
  LoadingOutlined,
  MinusCircleFilled,
  PauseCircleFilled,
} from '@ant-design/icons';
import { SessionStatus } from '../types';
import { sessionStatusDisplay } from '../lib/format';

const statusIcons: Record<SessionStatus, React.ReactNode> = {
  starting: <LoadingOutlined spin />,
  idle: <MinusCircleFilled />,
  running: <CheckCircleFilled />,
  suspended: <PauseCircleFilled />,
  error: <CloseCircleFilled />,
  exited: <MinusCircleFilled />,
  unknown: <MinusCircleFilled />,
};

export function StatusTag({ status }: { status: SessionStatus }) {
  const display = sessionStatusDisplay[status] || sessionStatusDisplay.unknown;
  return (
    <Tag
      color={display.tagColor}
      icon={statusIcons[status] || statusIcons.unknown}
      className="status-tag"
    >
      {display.label}
    </Tag>
  );
}
