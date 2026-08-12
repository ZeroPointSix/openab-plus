import { LinkOutlined, ReadOutlined } from '@ant-design/icons';
import { Button, Empty, Space, Spin, Typography } from 'antd';
import { formatDateTime, formatRelativeTime } from '../../lib/format';
import { SessionSnapshot } from '../../types';
import { EntityMark } from '../EntityMark';
import { StatusTag } from '../StatusTag';

interface SessionMainPanelProps {
  session?: SessionSnapshot;
  loading?: boolean;
  hasSelection: boolean;
}

export function SessionMainPanel({
  session,
  loading,
  hasSelection,
}: SessionMainPanelProps) {
  return (
    <main className="workbench-panel workbench-main" aria-label="会话主视图">
      {loading ? (
        <div className="workbench-main-loading">
          <Spin size="large" />
        </div>
      ) : !hasSelection ? (
        <div className="workbench-main-empty">
          <Empty description="从左侧选择一条 Agent 会话开始观测" />
        </div>
      ) : !session ? (
        <div className="workbench-main-empty">
          <Empty description="未找到该会话，可能已结束或 ID 无效" />
        </div>
      ) : (
        <>
          <div className="workbench-status-bar">
            <div className="workbench-status-primary">
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
                模型 {session.model || '-'}
              </Typography.Text>
              <Typography.Text type="secondary">
                更新 {formatRelativeTime(session.updated_at)}
              </Typography.Text>
              {session.external_url ? (
                <Button
                  size="small"
                  icon={<LinkOutlined />}
                  href={session.external_url}
                  target="_blank"
                  rel="noreferrer"
                >
                  打开来源
                </Button>
              ) : null}
            </Space>
          </div>

          <div className="workbench-main-content">
            <section className="workbench-placeholder" aria-label="活动流占位">
              <span className="workbench-placeholder-icon" aria-hidden="true">
                <ReadOutlined />
              </span>
              <Typography.Title level={4}>活动流（W4 接入）</Typography.Title>
              <Typography.Paragraph type="secondary">
                此处将展示 Agent 执行过程中的完整 Transcript 与工具调用活动流。
                当前版本仅提供只读观测入口，不提供发送、插话或停止控制。
              </Typography.Paragraph>
              <Typography.Text type="secondary">
                最近更新 {formatDateTime(session.updated_at)}
              </Typography.Text>
            </section>
          </div>
        </>
      )}

      <footer className="workbench-readonly-bar" aria-label="只读观测提示">
        <span className="workbench-readonly-lock" aria-hidden="true">
          🔒
        </span>
        <div className="workbench-readonly-copy">
          <Typography.Text strong>只读观测</Typography.Text>
          <Typography.Text type="secondary">
            本页不提供发送、插话、停止或参数修改 · SSE 连接态见顶部导航
          </Typography.Text>
        </div>
      </footer>
    </main>
  );
}
