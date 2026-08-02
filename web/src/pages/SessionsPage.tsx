import { PageContainer } from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
import { adminApi } from '../lib/api';
import { SessionTable } from '../components/SessionTable';

export function SessionsPage() {
  const sessionsQuery = useQuery({
    queryKey: ['sessions'],
    queryFn: adminApi.sessions,
    refetchInterval: 30_000,
  });

  return (
    <PageContainer
      title="会话"
      subTitle="跨平台 Agent 会话与实时状态"
      className="page-container"
    >
      <SessionTable
        sessions={sessionsQuery.data || []}
        loading={sessionsQuery.isLoading}
        onReload={() => void sessionsQuery.refetch()}
        title="全部会话"
      />
    </PageContainer>
  );
}
