import { BranchesOutlined, ProfileOutlined } from '@ant-design/icons';
import { Alert, Descriptions, Empty, Space, Tabs, Typography } from 'antd';
import { agentDisplayName, formatDateTime } from '../../lib/format';
import { SessionSnapshot, TranscriptEntry } from '../../types';
import { StatusTag } from '../StatusTag';
import { SessionEventLog } from './SessionEventLog';

interface SessionInspectorProps {
  session?: SessionSnapshot;
  entries: TranscriptEntry[];
  hasSelection: boolean;
}

export function SessionInspector({
  session,
  entries,
  hasSelection,
}: SessionInspectorProps) {
  if (!hasSelection) {
    return (
      <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
        <div className="workbench-inspector-empty">
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description="选择会话后查看元数据与执行日志"
          />
        </div>
      </aside>
    );
  }

  if (!session) {
    return (
      <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
        <div className="workbench-inspector-empty">
          <Empty description="会话不可用" />
        </div>
      </aside>
    );
  }

  // ZER-715 P1-4: the standalone 告警 tab duplicated the inline alert already
  // rendered in the main panel and was empty in practice. Profile config
  // warnings now live next to the metadata they describe.
  const configErrors = session.profile_config_errors || [];

  const tabItems = [
    {
      key: 'metadata',
      label: (
        <span className="inspector-tab-label">
          <ProfileOutlined />
          元数据
        </span>
      ),
      children: (
        <div className="inspector-tab-body">
          <div className="inspector-section-heading">
            <StatusTag status={session.status} />
          </div>
          {configErrors.map((error) => (
            <Alert
              key={error.config_id}
              type="warning"
              showIcon
              message={'Profile 配置告警 · ' + error.config_id}
              description={error.error}
              className="inspector-alert"
            />
          ))}
          <Descriptions column={1} size="small" colon={false}>
            <Descriptions.Item label="Agent">
              {agentDisplayName(session.agent)}
            </Descriptions.Item>
            <Descriptions.Item label="平台">
              {session.source.platform || '-'}
            </Descriptions.Item>
            <Descriptions.Item label="Thread">
              <Typography.Text copyable code>
                {session.source.thread_id || '-'}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="工作目录">
              <Typography.Text copyable code>
                {session.workdir || '-'}
              </Typography.Text>
            </Descriptions.Item>
            <Descriptions.Item label="Profile">
              <Space size={6}>
                <span>{session.profile_name || session.profile_id || '-'}</span>
                {session.profile_status === 'deleted' ? (
                  <Typography.Text type="danger">已删除</Typography.Text>
                ) : null}
              </Space>
            </Descriptions.Item>
            <Descriptions.Item label="模型">
              {session.model || '--'}
            </Descriptions.Item>
            <Descriptions.Item label="Thinking">
              {session.reasoning_effort || '--'}
            </Descriptions.Item>
            <Descriptions.Item label="元数据来源">
              {session.metadata_source === 'acp'
                ? 'ACP 运行时'
                : session.metadata_source === 'configured'
                  ? '配置值'
                  : '--'}
            </Descriptions.Item>
            <Descriptions.Item label="创建时间">
              {formatDateTime(session.created_at)}
            </Descriptions.Item>
            <Descriptions.Item label="更新时间">
              {formatDateTime(session.updated_at)}
            </Descriptions.Item>
          </Descriptions>
        </div>
      ),
    },
    {
      key: 'events',
      label: (
        <span className="inspector-tab-label">
          <BranchesOutlined />
          日志
          {entries.length ? (
            <span className="inspector-tab-badge">{entries.length}</span>
          ) : null}
        </span>
      ),
      children: (
        <div className="inspector-tab-body">
          <SessionEventLog entries={entries} />
        </div>
      ),
    },
  ];

  return (
    <aside className="workbench-panel workbench-inspector" aria-label="会话详情">
      <Tabs
        className="workbench-inspector-tabs"
        items={tabItems}
        size="small"
      />
    </aside>
  );
}
