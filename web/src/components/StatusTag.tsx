import { Tag } from 'antd';
import { SessionStatus } from '../types';
import { sessionStatusDisplay } from '../lib/sessionStatus';

export function StatusTag({ status }: { status: SessionStatus }) {
  const config = sessionStatusDisplay(status);
  return (
    <Tag color={config.color} icon={config.icon} className="status-tag">
      {config.label}
    </Tag>
  );
}
