import {
  ArrowLeftOutlined,
  BranchesOutlined,
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
} from 'antd';
import { useQuery } from '@tanstack/react-query';
import { useNavigate, useParams } from 'react-router-dom';
import { adminApi } from '../lib/api';
import { formatDateTime } from '../lib/format';
import { StatusTag } from '../components/StatusTag';
import { SessionActivityFeed } from '../components/activity/SessionActivityFeed';
import { mockTranscript } from '../components/activity/mockTranscript';

export function SessionDetailPage() {
  const params = useParams<{ id: string }>();
  const sessionId = params.id || '';
  const navigate = useNavigate();
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
            key="external"
            type="primary"
            icon={<LinkOutlined />}
            href={session.external_url}
            target="_blank"
            rel="noreferrer"
          >
            打开来源
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
                </Typography.Text>
              </div>
            </div>
          </div>
          <SessionActivityFeed entries={mockTranscript} source="mock" />
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
