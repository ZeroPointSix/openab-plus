import {
  CodeOutlined,
  FileProtectOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
} from '@ant-design/icons';
import {
  PageContainer,
  ProColumns,
  ProTable,
} from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
import {
  Alert,
  Button,
  Descriptions,
  Tag,
  Typography,
} from 'antd';
import { adminApi } from '../lib/api';
import { ConfigMetadata } from '../types';
import { formatDateTime } from '../lib/format';

const policyLabels = {
  runtime: { label: '实时生效', color: 'green' },
  new_session: { label: '新会话生效', color: 'blue' },
  restart_required: { label: '重启生效', color: 'orange' },
} as const;

export function ConfigPage() {
  const configQuery = useQuery({
    queryKey: ['config'],
    queryFn: adminApi.config,
  });
  const statusQuery = useQuery({
    queryKey: ['configStatus'],
    queryFn: adminApi.configStatus,
  });

  const refresh = () => {
    void Promise.all([configQuery.refetch(), statusQuery.refetch()]);
  };
  const status = statusQuery.data;
  const metadataColumns: ProColumns<ConfigMetadata>[] = [
    {
      title: '字段路径',
      dataIndex: 'path',
      render: (_, record) => <Typography.Text code>{record.path}</Typography.Text>,
    },
    {
      title: '生效策略',
      dataIndex: 'apply_policy',
      width: 150,
      render: (_, record) => {
        const policy = policyLabels[record.apply_policy];
        return <Tag color={policy.color}>{policy.label}</Tag>;
      },
    },
    {
      title: '敏感字段',
      dataIndex: 'secret',
      width: 120,
      render: (_, record) =>
        record.secret ? (
          <Tag color="red" icon={<SafetyCertificateOutlined />}>
            已脱敏
          </Tag>
        ) : (
          <Tag>否</Tag>
        ),
    },
  ];

  return (
    <PageContainer
      title="Gateway 配置"
      subTitle="当前配置与运行时应用状态（只读）"
      className="page-container"
      extra={[
        <Button
          key="refresh"
          icon={<ReloadOutlined />}
          onClick={refresh}
          loading={configQuery.isFetching || statusQuery.isFetching}
        >
          刷新
        </Button>,
      ]}
    >
      {status?.pending_restart.length ? (
        <Alert
          type="warning"
          showIcon
          message="存在等待重启生效的配置"
          description={status.pending_restart.join('、')}
          className="config-alert"
        />
      ) : null}
      {status?.last_validation && !status.last_validation.ok ? (
        <Alert
          type="error"
          showIcon
          message="最近一次配置校验未通过"
          description={status.last_validation.errors
            .map((error) => error.path + ': ' + error.message)
            .join('；')}
          className="config-alert"
        />
      ) : null}

      <section className="config-status-band">
        <div className="panel-heading">
          <div className="panel-heading-title">
            <span className="panel-icon blue" aria-hidden="true">
              <FileProtectOutlined />
            </span>
            <div>
              <Typography.Title level={4}>配置状态</Typography.Title>
              <Typography.Text type="secondary">
                运行时加载、校验与回滚能力
              </Typography.Text>
            </div>
          </div>
          <Tag color={status?.rollback_available ? 'blue' : 'default'}>
            {status?.rollback_available ? '可回滚' : '无回滚快照'}
          </Tag>
        </div>
        <Descriptions column={{ xs: 1, sm: 2, lg: 4 }} size="small" colon={false}>
          <Descriptions.Item label="配置路径">
            <Typography.Text code>{status?.config_path || '-'}</Typography.Text>
          </Descriptions.Item>
          <Descriptions.Item label="最近保存">
            {formatDateTime(status?.last_saved_at)}
          </Descriptions.Item>
          <Descriptions.Item label="加载哈希">
            <Typography.Text copyable code>
              {status?.last_loaded_hash || '-'}
            </Typography.Text>
          </Descriptions.Item>
          <Descriptions.Item label="校验状态">
            {status?.last_validation ? (
              <Tag color={status.last_validation.ok ? 'green' : 'red'}>
                {status.last_validation.ok ? '通过' : '未通过'}
              </Tag>
            ) : (
              <Tag>暂无记录</Tag>
            )}
          </Descriptions.Item>
        </Descriptions>
      </section>

      <div className="config-grid">
        <section className="detail-panel config-document">
          <div className="panel-heading">
            <div className="panel-heading-title">
              <span className="panel-icon dark" aria-hidden="true">
                <CodeOutlined />
              </span>
              <div>
                <Typography.Title level={4}>当前配置</Typography.Title>
                <Typography.Text type="secondary">
                  敏感字段已脱敏展示
                </Typography.Text>
              </div>
            </div>
            <Tag color="blue">只读</Tag>
          </div>
          <pre aria-label="Gateway 当前配置">
            {JSON.stringify(configQuery.data?.values || {}, null, 2)}
          </pre>
        </section>
        <ProTable<ConfigMetadata>
          className="metadata-table"
          rowKey="path"
          columns={metadataColumns}
          dataSource={configQuery.data?.metadata || []}
          loading={configQuery.isLoading}
          search={false}
          pagination={{ defaultPageSize: 12, hideOnSinglePage: true }}
          cardBordered
          headerTitle="字段应用策略"
          options={false}
        />
      </div>
    </PageContainer>
  );
}
