import { useMemo, useState } from 'react';
import {
  AppstoreOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  FilterOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import {
  PageContainer,
  ProForm,
  ProFormDateTimeRangePicker,
  ProFormSelect,
} from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
import { Typography } from 'antd';
import { SessionFilters } from '../types';
import { adminApi } from '../lib/api';
import { filterSessions, sessionMetrics } from '../lib/session';
import { SessionTable } from '../components/SessionTable';

function normalizeUpdatedRange(value: unknown): [string, string] | undefined {
  if (!Array.isArray(value) || value.length !== 2) {
    return undefined;
  }

  const normalized = value.map((item) => {
    if (typeof item === 'string') {
      return item;
    }

    if (
      item &&
      typeof item === 'object' &&
      'toISOString' in item &&
      typeof item.toISOString === 'function'
    ) {
      return item.toISOString();
    }

    return undefined;
  });

  return normalized.every((item): item is string => Boolean(item))
    ? [normalized[0], normalized[1]]
    : undefined;
}

export function OverviewPage() {
  const [filters, setFilters] = useState<SessionFilters>({});
  const sessionsQuery = useQuery({
    queryKey: ['sessions'],
    queryFn: adminApi.sessions,
    refetchInterval: 30_000,
  });
  const profilesQuery = useQuery({
    queryKey: ['profiles'],
    queryFn: adminApi.profiles,
  });

  const sessions = sessionsQuery.data || [];
  const filteredSessions = useMemo(
    () => filterSessions(sessions, filters),
    [filters, sessions],
  );
  const metrics = sessionMetrics(filteredSessions);
  const platforms = [...new Set(sessions.map((item) => item.source.platform))];
  const profiles = profilesQuery.data?.profiles || [];

  const metricCards = [
    {
      key: 'total',
      label: '会话总数',
      value: metrics.total,
      caption: 'Gateway 接入的全部会话',
      icon: <AppstoreOutlined />,
      tone: 'blue',
    },
    {
      key: 'active',
      label: '活跃会话',
      value: metrics.active,
      caption: '空闲或运行中，可继续交互',
      icon: <CheckCircleOutlined />,
      tone: 'green',
    },
    {
      key: 'running',
      label: '运行中',
      value: metrics.running,
      caption: '正在处理任务的会话',
      icon: <ThunderboltOutlined />,
      tone: 'amber',
    },
    {
      key: 'failed',
      label: '失败',
      value: metrics.failed,
      caption: '启动或运行异常，需要关注',
      icon: <CloseCircleOutlined />,
      tone: 'red',
    },
  ];

  return (
    <PageContainer
      title="运行总览"
      subTitle="Gateway 会话与 Agent 运行状态"
      className="page-container"
    >
      <section className="metric-grid" aria-label="运行指标">
        {metricCards.map((card) => (
          <article key={card.key} className={'metric-card tone-' + card.tone}>
            <span className="metric-card-icon" aria-hidden="true">
              {card.icon}
            </span>
            <div className="metric-card-body">
              <span className="metric-card-label">{card.label}</span>
              <span className="metric-card-value">{card.value}</span>
              <span className="metric-card-caption">{card.caption}</span>
            </div>
          </article>
        ))}
      </section>

      <section className="overview-filter" aria-label="总览筛选">
        <div className="section-heading">
          <span className="section-heading-icon" aria-hidden="true">
            <FilterOutlined />
          </span>
          <div className="section-heading-text">
            <Typography.Text strong>筛选会话</Typography.Text>
            <Typography.Text type="secondary">
              按平台、状态、Profile 或更新时间过滤
            </Typography.Text>
          </div>
        </div>
        <ProForm
          layout="horizontal"
          submitter={false}
          onValuesChange={(_, values) =>
            setFilters({
              platform: values.platform,
              status: values.status,
              profile: values.profile,
              updatedRange: normalizeUpdatedRange(values.updatedRange),
            })
          }
        >
          <ProFormSelect
            name="platform"
            label="平台"
            allowClear
            width="sm"
            options={platforms.map((value) => ({ label: value, value }))}
          />
          <ProFormSelect
            name="status"
            label="状态"
            allowClear
            width="sm"
            options={[
              { label: '启动中', value: 'starting' },
              { label: '空闲', value: 'idle' },
              { label: '运行中', value: 'running' },
              { label: '已暂停', value: 'suspended' },
              { label: '失败', value: 'error' },
              { label: '已退出', value: 'exited' },
            ]}
          />
          <ProFormSelect
            name="profile"
            label="Profile"
            allowClear
            width="sm"
            options={profiles.map((profile) => ({
              label: profile.name || profile.id,
              value: profile.id,
            }))}
          />
          <ProFormDateTimeRangePicker
            name="updatedRange"
            label="更新时间"
            fieldProps={{ allowClear: true }}
          />
        </ProForm>
      </section>

      <SessionTable
        title="最近会话"
        sessions={filteredSessions}
        loading={sessionsQuery.isLoading}
        compact
        searchable={false}
        limit={8}
        onReload={() => void sessionsQuery.refetch()}
      />
    </PageContainer>
  );
}
