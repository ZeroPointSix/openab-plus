import {
  ArrowLeftOutlined,
  BranchesOutlined,
  CopyOutlined,
  LinkOutlined,
  ProfileOutlined,
} from '@ant-design/icons';
import { PageContainer } from '@ant-design/pro-components';
import {
  Alert,
  Button,
  Descriptions,
  Empty,
  Space,
  Spin,
  Typography,
  message,
} from 'antd';
import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { adminApi } from '../lib/api';
import { formatDateTime, sourcePlatformLabel } from '../lib/format';
import { StatusTag } from '../components/StatusTag';
import { SessionActivityFeed } from '../components/activity/SessionActivityFeed';
import { useSessionTranscript } from '../hooks/useSessionTranscript';
import { activityEntriesFromTranscript } from '../lib/transcript';

export function SessionDetailPage() {
  const params = useParams<{ id: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
  const transcript = useSessionTranscript(sessionId);
  const activityEntries = useMemo(
    () => activityEntriesFromTranscript(transcript.entries),
    [transcript.entries],
  );
  const sessionQuery = useQuery({
    queryKey: ['session', sessionId],
    queryFn: () => adminApi.session(sessionId),
    enabled: Boolean(sessionId),
  });
  if (sessionQuery.isLoading) {
    return (
      <div className="page-loading">
        <Spin size="large" />
      </div>
    );
  }

  if (!sessionQuery.data) {
    return (
      <PageContainer title="会话详情" className="page-container">
        <Empty description="未找到该会话">
          <Button onClick={() => navigate('/sessions')}>返回会话列表</Button>
        </Empty>
      </PageContainer>
    );
  }

  const session = sessionQuery.data;
  const metadataSourceLabel =
    session.metadata_source === 'acp'
      ? 'ACP 运行时'
      : session.metadata_source === 'configured'
        ? '配置值'
        : '未报告';
  const sourcePlatform = sourcePlatformLabel(session.source?.platform);

  const copySessionLink = async () => {
    if (!session.external_url) return;
    try {
      await navigator.clipboard.writeText(session.external_url);
      message.success('会话链接已复制');
    } catch {
      message.error('复制失败，请检查浏览器剪贴板权限');
    }
  };

  return (
    <PageContainer
      title={session.session_id}
      subTitle={session.agent || 'Agent 未报告'}
      className="page-container"
      extra={[
        <Button
          key="back"
          icon={<ArrowLeftOutlined />}
          onClick={() => navigate('/sessions')}
        >
          返回
        </Button>,
        session.source?.permalink ? (
          <Button
            key="source"
            type="primary"
            icon={<LinkOutlined />}
            href={session.source.permalink}
            target="_blank"
            rel="noreferrer"
          >
            {sourcePlatform ? '回到 ' + sourcePlatform : '打开来源'}
          </Button>
        ) : null,
        session.external_url ? (
          <Button
            key="copy-link"
            icon={<CopyOutlined />}
            onClick={copySessionLink}
          >
            复制会话链接
          </Button>
        ) : null,
      ]}
    >
      {session.last_error ? (
        <Alert
          type="error"
          showIcon
          message="会话异常"
          description={session.last_error}
          className="detail-alert"
        />
      ) : null}
      {transcript.recovery ? (
        <Alert
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
          className="detail-alert"
        />
      ) : null}
      <div className="session-detail-grid">
        <section className="detail-panel activity-panel">
          <div className="panel-heading">
            <div className="panel-heading-title">
              <span className="panel-icon blue" aria-hidden="true">
                <BranchesOutlined />
              </span>
              <div>
                <Typography.Title level={4}>Agent 活动流</Typography.Title>
                <Typography.Text type="secondary">
                  连续展示回复、思考、计划、工具调用、终端输出和文件编辑差异
                  {transcript.latencyMs !== undefined
                    ? ` · SSE 端到端延迟 ${transcript.latencyMs}ms${
                        transcript.latencyMs > 2_000 ? '（超过 2s 观测目标）' : ''
                      }`
                    : ''}
                </Typography.Text>
              </div>
            </div>
          </div>
          <SessionActivityFeed entries={activityEntries} source="live" />
        </section>

        <aside className="detail-panel metadata-panel">
          <div className="panel-heading">
            <div className="panel-heading-title">
              <span className="panel-icon violet" aria-hidden="true">
                <ProfileOutlined />
              </span>
              <div>
                <Typography.Title level={4}>会话元数据</Typography.Title>
                <Typography.Text type="secondary">
                  来源、工作目录与运行参数
                </Typography.Text>
              </div>
            </div>
            <StatusTag status={session.status} />
          </div>
          <Descriptions column={1} size="small" colon={false}>
            <Descriptions.Item label="Agent">
              {session.agent || '未报告'}
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
              {session.model || '未报告'}
            </Descriptions.Item>
            <Descriptions.Item label="Thinking">
              {session.reasoning_effort || '未报告'}
            </Descriptions.Item>
            <Descriptions.Item label="元数据来源">
              {metadataSourceLabel}
            </Descriptions.Item>
            <Descriptions.Item label="创建时间">
              {formatDateTime(session.created_at)}
            </Descriptions.Item>
            <Descriptions.Item label="更新时间">
              {formatDateTime(session.updated_at)}
            </Descriptions.Item>
          </Descriptions>
          {session.profile_config_errors?.length ? (
            <Alert
              type="warning"
              showIcon
              message="Profile 配置告警"
              description={session.profile_config_errors.map((error) => (
                <div key={error.config_id}>
                  {error.config_id}: {error.error}
                </div>
              ))}
            />
          ) : null}
        </aside>
      </div>
    </PageContainer>
  );
}
