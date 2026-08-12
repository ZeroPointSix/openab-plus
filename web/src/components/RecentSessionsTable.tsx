import { useMemo, useState } from 'react';
import { Button, Empty, Space, Tooltip, Typography, message } from 'antd';
import {
  DownloadOutlined,
  EyeOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import {
  ActionType,
  ProColumns,
  ProTable,
} from '@ant-design/pro-components';
import { useNavigate } from 'react-router-dom';
import { SessionFilters, SessionSnapshot } from '../types';
import { filterSessions } from '../lib/session';
import { formatDateTime, formatRelativeTime } from '../lib/format';
import { SESSION_STATUS_VALUE_ENUM } from '../lib/sessionStatus';
import { StatusTag } from './StatusTag';
import { EntityMark } from './EntityMark';

interface RecentSessionsTableProps {
  sessions: SessionSnapshot[];
  loading?: boolean;
  compact?: boolean;
  searchable?: boolean;
  onReload?: () => void;
  title?: string;
  limit?: number;
}

interface SearchValues {
  source?: { platform?: string };
  status?: string;
  profile_id?: string;
  agent?: string;
  updated_at?: [string, string];
}

export function RecentSessionsTable({
  sessions,
  loading,
  compact = false,
  searchable = true,
  onReload,
  title = '会话',
  limit,
}: RecentSessionsTableProps) {
  const navigate = useNavigate();
  const [filters, setFilters] = useState<SessionFilters & { agent?: string }>(
    {},
  );

  const dataSource = useMemo(() => {
    const filtered = filterSessions(sessions, filters).filter(
      (session) => !filters.agent || session.agent === filters.agent,
    );
    return typeof limit === 'number' ? filtered.slice(0, limit) : filtered;
  }, [filters, limit, sessions]);

  const enumFrom = (values: Array<string | undefined>) =>
    Object.fromEntries(
      [...new Set(values.filter(Boolean) as string[])].map((value) => [
        value,
        { text: value },
      ]),
    );

  const columns: ProColumns<SessionSnapshot>[] = [
    {
      title: '状态',
      dataIndex: 'status',
      width: 112,
      valueType: 'select',
      valueEnum: SESSION_STATUS_VALUE_ENUM,
      render: (_, record) => <StatusTag status={record.status} />,
    },
    {
      title: 'Agent',
      dataIndex: 'agent',
      width: 128,
      valueType: 'select',
      valueEnum: enumFrom(sessions.map((session) => session.agent)),
      render: (_, record) => (
        <Space size={8} className="agent-cell">
          <EntityMark name={record.agent} />
          <Typography.Text strong>
            {record.agent || '未报告'}
          </Typography.Text>
        </Space>
      ),
    },
    {
      title: '平台',
      dataIndex: ['source', 'platform'],
      width: 120,
      valueType: 'select',
      valueEnum: enumFrom(
        sessions.map((session) => session.source?.platform),
      ),
      render: (_, record) => record.source?.platform || '-',
    },
    {
      title: '工作目录',
      dataIndex: 'workdir',
      ellipsis: true,
      hideInSearch: true,
      render: (_, record) => (
        <Typography.Text code ellipsis={{ tooltip: record.workdir }}>
          {record.workdir || '-'}
        </Typography.Text>
      ),
    },
    {
      title: 'Profile',
      dataIndex: 'profile_id',
      width: 160,
      valueType: 'select',
      valueEnum: Object.fromEntries(
        sessions
          .filter((session) => session.profile_id)
          .map((session) => [
            session.profile_id as string,
            { text: session.profile_name || session.profile_id },
          ]),
      ),
      render: (_, record) => (
        <Space size={4}>
          <span>{record.profile_name || record.profile_id || '-'}</span>
          {record.profile_status === 'deleted' ? (
            <Typography.Text type="danger">已删除</Typography.Text>
          ) : null}
        </Space>
      ),
    },
    {
      title: '更新时间',
      dataIndex: 'updated_at',
      width: 178,
      valueType: 'dateTimeRange',
      render: (_, record) => (
        <Tooltip title={formatDateTime(record.updated_at)}>
          <span>{formatRelativeTime(record.updated_at)}</span>
        </Tooltip>
      ),
    },
    {
      title: '操作',
      valueType: 'option',
      width: 72,
      fixed: 'right',
      render: (_, record) => [
        <Tooltip title="查看详情" key="view">
          <Button
            type="text"
            icon={<EyeOutlined />}
            aria-label="查看会话详情"
            onClick={() =>
              navigate('/sessions/' + encodeURIComponent(record.session_id))
            }
          />
        </Tooltip>,
      ],
    },
  ];

  return (
    <ProTable<SessionSnapshot, SearchValues>
      className="session-table"
      rowKey="session_id"
      columns={columns}
      dataSource={dataSource}
      loading={loading}
      search={
        searchable
          ? {
              labelWidth: 'auto',
              span: { xs: 24, sm: 12, md: 8, lg: 6, xl: 6, xxl: 6 },
              defaultCollapsed: false,
            }
          : false
      }
      onSubmit={(values) =>
        setFilters({
          platform: values.source?.platform,
          status: values.status,
          profile: values.profile_id,
          agent: values.agent,
          updatedRange: values.updated_at,
        })
      }
      onReset={() => setFilters({})}
      pagination={
        compact
          ? false
          : {
              defaultPageSize: 20,
              showSizeChanger: true,
              showTotal: (total) => '共 ' + total + ' 条',
            }
      }
      scroll={{ x: 980 }}
      size={compact ? 'small' : 'middle'}
      options={{
        density: !compact,
        fullScreen: !compact,
        reload: onReload
          ? () => {
              onReload();
              return Promise.resolve();
            }
          : false,
        setting: !compact,
      }}
      toolBarRender={() => [
        <Button
          key="export"
          icon={<DownloadOutlined />}
          onClick={() => message.info('导出能力将在后续版本开放')}
        >
          导出
        </Button>,
        onReload ? (
          <Button
            key="reconcile"
            type="primary"
            icon={<ReloadOutlined />}
            onClick={onReload}
          >
            全量校准
          </Button>
        ) : null,
      ]}
      locale={{
        emptyText: (
          <Empty
            className="table-empty"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <Space direction="vertical" size={2}>
                <Typography.Text strong>暂无会话</Typography.Text>
                <Typography.Text type="secondary">
                  Agent 通过 Gateway 启动会话后，会实时显示在这里
                </Typography.Text>
              </Space>
            }
          />
        ),
      }}
      headerTitle={
        <Space size={10} align="center">
          <span>{title}</span>
          <span className="table-count">共 {dataSource.length} 条</span>
        </Space>
      }
      cardBordered
    />
  );
}
