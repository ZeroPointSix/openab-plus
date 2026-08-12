import {
  CopyOutlined,
  LinkOutlined,
  ProfileOutlined,
  UnorderedListOutlined,
} from '@ant-design/icons';
import { Alert, Button, Empty, Space, Spin, Typography, message } from 'antd';
import {
  streamStatusLabels,
  useStreamStatus,
} from '../../hooks/streamStatusContext';
import {
  formatRelativeTime,
  sourcePlatformLabel,
} from '../../lib/format';
import { ActivityEntry, SessionSnapshot } from '../../types';
import { SessionActivityFeed } from '../activity/SessionActivityFeed';
import { EntityMark } from '../EntityMark';
import { StatusTag } from '../StatusTag';

interface SessionMainPanelProps {
  session?: SessionSnapshot;
  activityEntries: ActivityEntry[];
  activityLoading?: boolean;
  activityError?: string;
  loading?: boolean;
  hasSelection: boolean;
  onOpenSidebar?: () => void;
  onOpenInspector?: () => void;
}

export function SessionMainPanel({
  session,
  activityEntries,
  activityLoading,
  activityError,
  loading,
  hasSelection,
  onOpenSidebar,
  onOpenInspector,
}: SessionMainPanelProps) {
  const streamStatus = useStreamStatus();
  const showSession = Boolean(hasSelection && session && !loading);
  const hasToggles = Boolean(onOpenSidebar || onOpenInspector);

  const copySessionLink = async () => {
    if (!session?.external_url) return;
    try {
      await navigator.clipboard.writeText(session.external_url);
      message.success('会话链接已复制');
    } catch {
      message.error('复制失败，请检查浏览器剪贴板权限');
    }
  };

  const sidebarToggle = onOpenSidebar ? (
    <Button
      type="text"
      size="small"
      className="workbench-status-toggle"
      icon={<UnorderedListOutlined />}
      aria-label="打开会话列表"
      onClick={onOpenSidebar}
    />
  ) : null;

  const inspectorToggle = onOpenInspector ? (
    <Button
      type="text"
      size="small"
      className="workbench-status-toggle"
      icon={<ProfileOutlined />}
      aria-label="打开会话详情"
      onClick={onOpenInspector}
    />
  ) : null;

  return (
    <main className="workbench-panel workbench-main" aria-label="会话主视图">
      {showSession && session ? (
        <div className="workbench-status-bar">
          <div className="workbench-status-primary">
            {sidebarToggle}
            <EntityMark name={session.agent} size={28} />
            <div className="workbench-status-copy">
              <Typography.Title level={5} className="workbench-status-title">
                {session.agent || '未知 Agent'}
              </Typography.Title>
              <Typography.Text type="secondary" code copyable>
                {session.session_id}
              </Typography.Text>
            </div>
          </div>
          <Space wrap size={[12, 8]} className="workbench-status-meta">
            <StatusTag status={session.status} />
            <Typography.Text type="secondary">
              Profile {session.profile_name || session.profile_id || '-'}
            </Typography.Text>
            <Typography.Text type="secondary">
              模型 {session.model || '-'}
            </Typography.Text>
            <Typography.Text
              type="secondary"
              className="workbench-status-workdir"
              ellipsis={{ tooltip: session.workdir || '-' }}
            >
              目录 <Typography.Text code>{session.workdir || '-'}</Typography.Text>
            </Typography.Text>
            <Typography.Text type="secondary">
              更新 {formatRelativeTime(session.updated_at)}
            </Typography.Text>
            {session.source?.permalink ? (
              <Button
                size="small"
                type="primary"
                icon={<LinkOutlined />}
                href={session.source.permalink}
                target="_blank"
                rel="noreferrer"
              >
                {sourcePlatformLabel(session.source.platform)
                  ? '回到 ' + sourcePlatformLabel(session.source.platform)
                  : '打开来源'}
              </Button>
            ) : null}
            {session.external_url ? (
              <Button
                size="small"
                icon={<CopyOutlined />}
                onClick={() => void copySessionLink()}
              >
                复制会话链接
              </Button>
            ) : null}
            {inspectorToggle}
          </Space>
        </div>
      ) : hasToggles ? (
        <div className="workbench-status-bar">
          <div className="workbench-status-primary">{sidebarToggle}</div>
          <Space wrap size={[12, 8]} className="workbench-status-meta">
            {inspectorToggle}
          </Space>
        </div>
      ) : null}

      <div className="workbench-main-content">
        {loading ? (
          <div className="workbench-main-loading">
            <Spin size="large" />
          </div>
        ) : !hasSelection ? (
          <div className="workbench-main-empty">
            <Empty description="选择一条 Agent 会话开始观测" />
          </div>
        ) : !session ? (
          <div className="workbench-main-empty">
            <Empty description="未找到该会话，可能已结束或 ID 无效" />
          </div>
        ) : activityLoading ? (
          <div className="workbench-main-loading">
            <Spin size="large" />
          </div>
        ) : activityError ? (
          <Alert
            className="workbench-activity-error"
            type="error"
            showIcon
            message="活动流加载失败"
            description={activityError}
          />
        ) : (
          <section className="activity-panel workbench-activity-panel">
            <SessionActivityFeed entries={activityEntries} />
          </section>
        )}
      </div>

      <footer className="workbench-readonly-bar" aria-label="只读观测提示">
        <span className="workbench-readonly-lock" aria-hidden="true">
          🔒
        </span>
        <div className="workbench-readonly-copy">
          <Typography.Text strong>只读观测</Typography.Text>
          <Typography.Text type="secondary">
            本页不提供发送、插话、停止或参数修改
          </Typography.Text>
        </div>
        <span
          className={'workbench-stream-status stream-status is-' + streamStatus}
        >
          <span className="stream-dot" aria-hidden="true" />
          <span className="stream-label">
            {streamStatusLabels[streamStatus]}
          </span>
        </span>
      </footer>
    </main>
  );
}
