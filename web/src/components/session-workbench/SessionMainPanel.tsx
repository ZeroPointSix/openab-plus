import {
  CopyOutlined,
  LinkOutlined,
  LockOutlined,
  ProfileOutlined,
  UnorderedListOutlined,
} from '@ant-design/icons';
import { Alert, Button, Empty, Space, Spin, Typography, message } from 'antd';
import { useMemo } from 'react';
import {
  agentDisplayName,
  sourcePlatformLabel,
  transcriptStatusLabel,
} from '../../lib/format';
import { activityEntriesFromTranscript } from '../../lib/transcript';
import { useSessionTranscript } from '../../hooks/useSessionTranscript';
import { SessionSnapshot } from '../../types';
import { EntityMark } from '../EntityMark';
import { SessionActivityFeed } from '../activity/SessionActivityFeed';
import { SessionRunIndicator } from './SessionRunIndicator';

interface SessionMainPanelProps {
  session?: SessionSnapshot;
  /**
   * ZER-715: the transcript stream is owned by the page so the activity feed
   * and the inspector log share a single SSE connection. Calling the hook in
   * both places would open the stream twice.
   */
  transcript: ReturnType<typeof useSessionTranscript>;
  loading?: boolean;
  loadError?: string;
  hasSelection: boolean;
  /**
   * Desktop collapse/expand or mobile open. The page owns the real action and
   * must pass matching labels so screen readers do not hear “打开” when the
   * button will fold an already-visible panel.
   */
  onToggleSidebar?: () => void;
  onToggleInspector?: () => void;
  sidebarToggleLabel?: string;
  inspectorToggleLabel?: string;
}

export function SessionMainPanel({
  session,
  transcript,
  loading,
  loadError,
  hasSelection,
  onToggleSidebar,
  onToggleInspector,
  sidebarToggleLabel = '打开会话列表',
  inspectorToggleLabel = '打开会话详情',
}: SessionMainPanelProps) {
  const activityEntries = useMemo(
    () => activityEntriesFromTranscript(transcript.entries),
    [transcript.entries],
  );
  const showSession = Boolean(hasSelection && session && !loading);
  const hasToggles = Boolean(onToggleSidebar || onToggleInspector);
  const sourcePlatform = sourcePlatformLabel(session?.source?.platform);
  const streamClass =
    transcript.status === 'recovery_needed' ? 'offline' : transcript.status;

  const sidebarToggle = onToggleSidebar ? (
    <Button
      type="text"
      size="small"
      className="workbench-status-toggle"
      icon={<UnorderedListOutlined />}
      aria-label={sidebarToggleLabel}
      onClick={onToggleSidebar}
    />
  ) : null;

  const inspectorToggle = onToggleInspector ? (
    <Button
      type="text"
      size="small"
      className="workbench-status-toggle"
      icon={<ProfileOutlined />}
      aria-label={inspectorToggleLabel}
      onClick={onToggleInspector}
    />
  ) : null;

  const copySessionLink = async () => {
    if (!session?.external_url) return;
    try {
      await navigator.clipboard.writeText(session.external_url);
      message.success('会话链接已复制');
    } catch {
      message.error('复制失败，请检查浏览器剪贴板权限');
    }
  };

  return (
    <main className="workbench-panel workbench-main" aria-label="会话主视图">
      {showSession && session ? (
        <div className="workbench-status-bar">
          <div className="workbench-status-primary">
            {sidebarToggle}
            <EntityMark name={session.agent} size={28} />
            <div className="workbench-status-copy">
              <Typography.Title level={5} className="workbench-status-title">
                {agentDisplayName(session.agent)}
              </Typography.Title>
              <Typography.Text type="secondary" ellipsis>
                {session.session_id}
              </Typography.Text>
            </div>
          </div>
          <Space wrap size={[8, 8]} className="workbench-status-meta">
            <SessionRunIndicator session={session} />
            <span className={'workbench-stream-status is-' + streamClass}>
              <span className="stream-dot" aria-hidden="true" />
              <span className="stream-label">
                {transcriptStatusLabel(transcript.status)}
              </span>
            </span>
            {session.source?.permalink ? (
              <Button
                size="small"
                type="text"
                icon={<LinkOutlined />}
                href={session.source.permalink}
                target="_blank"
                rel="noreferrer"
              >
                {sourcePlatform ? '回到 ' + sourcePlatform : '打开来源'}
              </Button>
            ) : null}
            {session.external_url ? (
              <Button
                size="small"
                type="text"
                icon={<CopyOutlined />}
                onClick={copySessionLink}
              >
                复制链接
              </Button>
            ) : null}
            {inspectorToggle}
          </Space>
        </div>
      ) : hasToggles ? (
        <div className="workbench-status-bar">
          <div className="workbench-status-primary">{sidebarToggle}</div>
          <Space wrap size={[8, 8]} className="workbench-status-meta">
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
            <Empty
              description={loadError || '未找到该会话，可能已结束或 ID 无效'}
            />
          </div>
        ) : (
          <section className="workbench-activity-stage" aria-label="Agent 活动流">
            {session.last_error ? (
              <Alert
                className="workbench-activity-alert"
                type="error"
                showIcon
                message="会话异常"
                description={session.last_error}
              />
            ) : null}
            {transcript.recovery ? (
              <Alert
                className="workbench-activity-alert"
                type="warning"
                showIcon
                message="活动流需要恢复"
                description={
                  <Space direction="vertical" size={8}>
                    <span>{transcript.recovery.message}</span>
                    {transcript.recovery.oldestSequence !== undefined ? (
                      <Typography.Text type="secondary">
                        可恢复记录起点：{transcript.recovery.oldestSequence}
                        {transcript.recovery.nextSequence !== undefined
                          ? `；服务端下一序号：${transcript.recovery.nextSequence}`
                          : ''}
                      </Typography.Text>
                    ) : null}
                    <Button type="primary" size="small" onClick={transcript.reload}>
                      重新拉取 transcript 快照
                    </Button>
                  </Space>
                }
              />
            ) : null}
            <SessionActivityFeed
              entries={activityEntries}
              streamStatus={transcript.status}
            />
          </section>
        )}
      </div>

      <footer className="workbench-readonly-bar" aria-label="只读观测提示">
        <LockOutlined className="workbench-readonly-lock" aria-hidden="true" />
        <div className="workbench-readonly-copy">
          <Typography.Text strong>只读观测</Typography.Text>
          <Typography.Text type="secondary">
            本页不提供发送、插话、停止或参数修改
          </Typography.Text>
        </div>
      </footer>
    </main>
  );
}
