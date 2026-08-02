import { Tag } from 'antd';
import {
  CheckCircleFilled,
  CloseCircleFilled,
  LoadingOutlined,
  MinusCircleFilled,
  PauseCircleFilled,
} from '@ant-design/icons';
import { SessionStatus } from '../types';
import { statusLabels } from '../lib/format';

const statusConfig: Record<
  SessionStatus,
  { color: string; icon: React.ReactNode }
> = {
  starting: { color: 'processing', icon: <LoadingOutlined spin /> },
  idle: { color: 'default', icon: <MinusCircleFilled /> },
  running: { color: 'success', icon: <CheckCircleFilled /> },
  suspended: { color: 'warning', icon: <PauseCircleFilled /> },
  error: { color: 'error', icon: <CloseCircleFilled /> },
  exited: { color: 'default', icon: <MinusCircleFilled /> },
  unknown: { color: 'default', icon: <MinusCircleFilled /> },
};

export function StatusTag({ status }: { status: SessionStatus }) {
  const config = statusConfig[status] || statusConfig.unknown;
  return (
    <Tag color={config.color} icon={config.icon} className="status-tag">
      {statusLabels[status] || statusLabels.unknown}
    </Tag>
  );
}
