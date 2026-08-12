import { useEffect } from 'react';
import {
  ArrowLeftOutlined,
  BranchesOutlined,
  ClockCircleOutlined,
  ExclamationCircleFilled,
  LinkOutlined,
  ProfileOutlined,
} from '@ant-design/icons';
import { PageContainer } from '@ant-design/pro-components';
import {
  Alert,
  Button,
  Descriptions,
  Empty,
  message,
  Space,
  Spin,
  Timeline,
  Typography,
} from 'antd';
import {
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { adminApi } from '../lib/api';
import {
  eventLabel,
  formatDateTime,
  sessionStatusDisplay,
} from '../lib/format';
import { initialTimeline } from '../lib/session';
import { SessionTimelineItem } from '../types';
import { StatusTag } from '../components/StatusTag';

export function SessionDetailPage() {
  const params = useParams<{ id: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const sessionQuery = useQuery({
    queryKey: ['session', sessionId],
    queryFn: () => adminApi.session(sessionId),
    enabled: Boolean(sessionId),
  });
  const timelineQuery = useQuery<SessionTimelineItem[]>({
    queryKey: ['sessionTimeline', sessionId],
    queryFn: async () => [],
    enabled: false,
    initialData: [],
  });

  useEffect(() => {
    if (!sessionQuery.data) return;
    queryClient.setQueryData<SessionTimelineItem[]>(
      ['sessionTimeline', sessionId],
      (current = []) =>
        current.length ? current : initialTimeline(sessionQuery.data),
    );
  }, [queryClient, sessionId, sessionQuery.data]);

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
  const timeline = timelineQuery.data || [];
  const sourcePlatformLabel =
    session.source.platform === 'slack'
      ? 'Slack'
      : session.source.platform === 'discord'
        ? 'Discord'
        : session.source.platform;
  const copySessionLink = async () => {
    if (!session.external_url) return;
    try {
      await navigator.clipboard.writeText(session.external_url);
      message.success('会话链接已复制');
    } catch {
      message.error('复制会话链接失败');
    }
  };
  const metadataSourceLabel =
    session.metadata_source === 'acp'
      ? 'ACP 运行时'
      : session.metadata_source === 'configured'
        ? '配置值'
        : '未报告';

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
        session.external_url ? (
          <Button
            key="copy-session-link"
            type="primary"
            icon={<LinkOutlined />}
            onClick={() => void copySessionLink()}
          >
            复制会话链接
          </Button>
        ) : null,
        session.source.permalink ? (
          <Button
            key="source-permalink"
            icon={<LinkOutlined />}
            href={session.source.permalink}
            target="_blank"
            rel="noreferrer"
          >
            回到 {sourcePlatformLabel}
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
      <div className="session-detail-grid">
        <section className="detail-panel timeline-panel">
          <div className="panel-heading">
            <div className="panel-heading-title">
              <span className="panel-icon blue" aria-hidden="true">
                <BranchesOutlined />
              </span>
              <div>
                <Typography.Title level={4}>事件时间线</Typography.Title>
                <Typography.Text type="secondary">
                  实时状态事件，最多保留当前会话最近 60 条
                </Typography.Text>
              </div>
            </div>
          </div>
          {timeline.length ? (
            <Timeline
              items={[...timeline].reverse().map((item) => ({
                color:
                  (sessionStatusDisplay[item.status] || sessionStatusDisplay.unknown)
                    .timelineColor,
                dot:
                  item.status === 'error' ? (
                    <ExclamationCircleFilled />
                  ) : (
                    <ClockCircleOutlined />
                  ),
                children: (
                  <div className="timeline-entry">
                    <div className="timeline-title">
                      <Typography.Text strong>
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
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无事件" />
          )}
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
