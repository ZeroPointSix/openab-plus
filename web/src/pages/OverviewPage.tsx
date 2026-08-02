import { useMemo, useState } from 'react';
import {
  AppstoreOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import {
  PageContainer,
  ProForm,
  ProFormDateTimeRangePicker,
  ProFormSelect,
  StatisticCard,
} from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
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

  return (
    <PageContainer
      title="运行总览"
      subTitle="Gateway 会话与 Agent 运行状态"
      className="page-container"
    >
      <section className="metric-strip" aria-label="运行指标">
        <StatisticCard.Group direction="row">
          <StatisticCard
            statistic={{
              title: '会话总数',
              value: metrics.total,
              icon: <AppstoreOutlined className="metric-icon blue" />,
            }}
          />
          <StatisticCard
            statistic={{
              title: '活跃会话',
              value: metrics.active,
              icon: <CheckCircleOutlined className="metric-icon green" />,
            }}
          />
          <StatisticCard
            statistic={{
              title: '运行中',
              value: metrics.running,
              icon: <ThunderboltOutlined className="metric-icon amber" />,
            }}
          />
          <StatisticCard
            statistic={{
              title: '失败',
              value: metrics.failed,
              icon: <CloseCircleOutlined className="metric-icon red" />,
            }}
          />
        </StatisticCard.Group>
      </section>

      <section className="overview-filter" aria-label="总览筛选">
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
