import {
  CopyOutlined,
  LinkOutlined,
  LockOutlined,
  ProfileOutlined,
  UnorderedListOutlined,
} from '@ant-design/icons';
import { Alert, Button, Empty, Space, Spin, Typography, message } from 'antd';
import { useMemo } from 'react';
import { formatRelativeTime, sourcePlatformLabel } from '../../lib/format';
import { activityEntriesFromTranscript } from '../../lib/transcript';
import { useSessionTranscript } from '../../hooks/useSessionTranscript';
import { SessionSnapshot } from '../../types';
import { EntityMark } from '../EntityMark';
import { StatusTag } from '../StatusTag';
import { SessionActivityFeed } from '../activity/SessionActivityFeed';

interface SessionMainPanelProps {
  session?: SessionSnapshot;
  loading?: boolean;
  hasSelection: boolean;
  onOpenSidebar?: () => void;
  onOpenInspector?: () => void;
}

const transcriptStatusLabels = {
  loading: '正在加载历史',
  connecting: '正在连接',
  live: '实时连接',
  reconnecting: '正在重连',
  offline: '离线',
  recovery_needed: '需要恢复',
} as const;

export function SessionMainPanel({
  session,
  loading,
  hasSelection,
  onOpenSidebar,
  onOpenInspector,
}: SessionMainPanelProps) {
  const transcript = useSessionTranscript(session?.session_id || '');
  const activityEntries = useMemo(
    () => activityEntriesFromTranscript(transcript.entries),
    [transcript.entries],
  );
  const showSession = Boolean(hasSelection && session && !loading);
  const hasToggles = Boolean(onOpenSidebar || onOpenInspector);
  const sourcePlatform = sourcePlatformLabel(session?.source?.platform);

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
            <Typography.Text type="secondary">模型 {session.model || '-'}</Typography.Text>
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
                icon={<LinkOutlined />}
                href={session.source.permalink}
                target="_blank"
                rel="noreferrer"
              >
                {sourcePlatform ? '回到 ' + sourcePlatform : '打开来源'}
              </Button>
            ) : null}
            {session.external_url ? (
              <Button size="small" icon={<CopyOutlined />} onClick={copySessionLink}>
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
            <div className="workbench-activity-heading">
              <div>
                <Typography.Title level={4}>Agent 活动流</Typography.Title>
                <Typography.Text type="secondary">
                  连续展示回复、思考、计划、工具调用、终端输出和文件编辑差异
                </Typography.Text>
              </div>
              <Typography.Text type="secondary">仅观测</Typography.Text>
            </div>
            <SessionActivityFeed entries={activityEntries} source="live" />
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
        <span
          className={
            'workbench-stream-status stream-status is-' +
            (transcript.status === 'recovery_needed' ? 'offline' : transcript.status)
          }
        >
          <span className="stream-dot" aria-hidden="true" />
          <span className="stream-label">{transcriptStatusLabels[transcript.status]}</span>
        </span>
      </footer>
    </main>
  );
}
