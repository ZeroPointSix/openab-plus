import { useEffect, useMemo, useState } from 'react';
import {
  CheckCircleOutlined,
  DeleteOutlined,
  EditOutlined,
  PlusOutlined,
  StarFilled,
  StarOutlined,
} from '@ant-design/icons';
import {
  DrawerForm,
  PageContainer,
  ProColumns,
  ProForm,
  ProFormDependency,
  ProFormDigit,
  ProFormGroup,
  ProFormList,
  ProFormSelect,
  ProFormSwitch,
  ProFormText,
  ProTable,
} from '@ant-design/pro-components';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Button,
  Empty,
  Form,
  Popconfirm,
  Space,
  Tabs,
  Tag,
  Tooltip,
  Typography,
  message,
} from 'antd';
import { adminApi, ApiError } from '../lib/api';
import { EntityMark } from '../components/EntityMark';
import {
  normalizeProfilePayload,
  profileToForm,
  ProfileFormValues,
} from '../lib/profile';
import {
  AgentConfigField,
  AgentProfile,
  ProfileValidationResult,
} from '../types';
import { formatDateTime } from '../lib/format';

const COMMON_AGENTS = [
  'codex',
  'claude',
  'gemini',
  'opencode',
  'kiro',
  'cursor',
  'hermes',
];

function DynamicConfigFields({
  agentType,
}: {
  agentType?: string;
}) {
  const schemaQuery = useQuery({
    queryKey: ['profileSchema', agentType],
    queryFn: () => adminApi.profileSchema(agentType as string),
    enabled: Boolean(agentType),
  });

  if (!agentType || !schemaQuery.data?.fields.length) return null;

  const fieldFor = (field: AgentConfigField) => {
    const name = ['config_options', field.id];
    const common = {
      key: field.id,
      name,
      label: field.label || field.id,
      tooltip: field.dynamic
        ? '该选项由当前 Agent 运行时动态提供'
        : undefined,
    };
    if (field.options?.length) {
      return (
        <ProFormSelect
          {...common}
          options={field.options.map((value) => ({ label: value, value }))}
          allowClear
        />
      );
    }
    if (['boolean', 'bool'].includes(field.kind)) {
      return (
        <ProFormSelect
          {...common}
          options={[
            { label: 'true', value: 'true' },
            { label: 'false', value: 'false' },
          ]}
          allowClear
        />
      );
    }
    if (['number', 'integer'].includes(field.kind)) {
      return <ProFormDigit {...common} fieldProps={{ precision: 0 }} />;
    }
    return <ProFormText {...common} />;
  };

  return (
    <section className="dynamic-config-section">
      <div className="form-section-heading">
        <div>
          <Typography.Text strong>Agent 配置项</Typography.Text>
          <Typography.Text type="secondary">
            来源：{schemaQuery.data.source}
          </Typography.Text>
        </div>
        <Tag color="blue">{schemaQuery.data.fields.length} 项</Tag>
      </div>
      <ProFormGroup>{schemaQuery.data.fields.map(fieldFor)}</ProFormGroup>
    </section>
  );
}

function validationText(result: ProfileValidationResult): string {
  if (result.ok) return 'Profile 校验通过';
  return result.errors
    .map((error) => error.path + ': ' + error.message)
    .join('；');
}

export function ProfilesPage() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<AgentProfile | undefined>();
  const [form] = Form.useForm<ProfileFormValues>();
  const agentType = Form.useWatch('agent_type', form);

  const profilesQuery = useQuery({
    queryKey: ['profiles'],
    queryFn: adminApi.profiles,
  });
  const agentsQuery = useQuery({
    queryKey: ['agents'],
    queryFn: adminApi.agents,
  });
  const profiles = profilesQuery.data?.profiles || [];
  const defaultProfile = profilesQuery.data?.default_profile;
  const agentTypes = useMemo(
    () =>
      [
        ...new Set([
          ...COMMON_AGENTS,
          ...(agentsQuery.data || []).map((agent) => agent.agent_type),
          ...profiles.map((profile) => profile.agent_type),
        ]),
      ].sort(),
    [agentsQuery.data, profiles],
  );

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['profiles'] }),
      queryClient.invalidateQueries({ queryKey: ['agents'] }),
    ]);
  };

  const saveMutation = useMutation({
    mutationFn: async (values: ProfileFormValues) => {
      const profile = normalizeProfilePayload(values);
      if (editing) {
        return adminApi.updateProfile(editing.id, profile);
      }
      return adminApi.createProfile(profile);
    },
    onSuccess: async () => {
      message.success(editing ? 'Profile 已更新' : 'Profile 已创建');
      setOpen(false);
      await refresh();
    },
    onError: (error) => {
      message.error(
        error instanceof ApiError ? error.message : '保存 Profile 失败',
      );
    },
  });

  const openEditor = (profile?: AgentProfile) => {
    setEditing(profile);
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    form.resetFields();
    form.setFieldsValue(
      profileToForm(
        editing || {
          id: '',
          name: '',
          agent_type: agentTypes[0] || 'codex',
          enabled: true,
          workdir_strategy: 'system_default',
          recovery_strategy: 'resume_session',
        },
      ),
    );
  }, [agentTypes, editing, form, open]);

  const columns: ProColumns<AgentProfile>[] = [
    {
      title: 'Profile',
      dataIndex: 'name',
      render: (_, profile) => (
        <Space size={10} className="profile-cell">
          <EntityMark name={profile.name || profile.id} size={30} />
          <Space direction="vertical" size={0}>
            <Space size={6}>
              <Typography.Text strong>{profile.name}</Typography.Text>
              {profile.id === defaultProfile ? (
                <Tag color="gold" icon={<StarFilled />}>
                  默认
                </Tag>
              ) : null}
            </Space>
            <Typography.Text type="secondary" code>
              {profile.id}
            </Typography.Text>
          </Space>
        </Space>
      ),
    },
    {
      title: 'Agent',
      dataIndex: 'agent_type',
      width: 140,
      valueType: 'select',
      valueEnum: Object.fromEntries(
        agentTypes.map((value) => [value, { text: value }]),
      ),
      render: (_, profile) => <Tag>{profile.agent_type}</Tag>,
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      width: 110,
      valueType: 'select',
      valueEnum: {
        true: { text: '启用' },
        false: { text: '停用' },
      },
      render: (_, profile) =>
        profile.enabled ? (
          <Tag color="success">启用</Tag>
        ) : (
          <Tag>停用</Tag>
        ),
    },
    {
      title: '默认模型',
      dataIndex: 'default_model',
      width: 160,
      hideInSearch: true,
      render: (_, profile) => profile.default_model || '-',
    },
    {
      title: '工作目录策略',
      dataIndex: 'workdir_strategy',
      width: 170,
      hideInSearch: true,
    },
    {
      title: '更新时间',
      dataIndex: 'updated_at',
      valueType: 'dateTime',
      width: 180,
      hideInSearch: true,
      render: (_, profile) => formatDateTime(profile.updated_at),
    },
    {
      title: '操作',
      valueType: 'option',
      width: 180,
      fixed: 'right',
      render: (_, profile) => [
        <Tooltip title="编辑" key="edit">
          <Button
            type="text"
            icon={<EditOutlined />}
            aria-label="编辑 Profile"
            onClick={() => openEditor(profile)}
          />
        </Tooltip>,
        <Tooltip title="校验" key="validate">
          <Button
            type="text"
            icon={<CheckCircleOutlined />}
            aria-label="校验 Profile"
            onClick={async () => {
              try {
                const result = await adminApi.validateProfile(profile.id);
                result.ok
                  ? message.success(validationText(result))
                  : message.warning(validationText(result));
              } catch {
                message.error('Profile 校验失败');
              }
            }}
          />
        </Tooltip>,
        <Tooltip title="设为默认" key="default">
          <Button
            type="text"
            icon={
              profile.id === defaultProfile ? <StarFilled /> : <StarOutlined />
            }
            aria-label="设为默认 Profile"
            disabled={profile.id === defaultProfile}
            onClick={async () => {
              await adminApi.setDefaultProfile(profile.id);
              message.success('默认 Profile 已更新');
              await refresh();
            }}
          />
        </Tooltip>,
        <Popconfirm
          key="delete"
          title="删除 Profile"
          description="现有会话会保留快照，新会话将无法再选择该 Profile。"
          okButtonProps={{ danger: true }}
          onConfirm={async () => {
            await adminApi.deleteProfile(profile.id);
            message.success('Profile 已删除');
            await refresh();
          }}
        >
          <Tooltip title="删除">
            <Button
              type="text"
              danger
              icon={<DeleteOutlined />}
              aria-label="删除 Profile"
            />
          </Tooltip>
        </Popconfirm>,
      ],
    },
  ];

  return (
    <PageContainer
      title="Agent Profile"
      subTitle="Agent 启动参数、运行策略与动态配置"
      className="page-container"
      extra={[
        defaultProfile ? (
          <Button
            key="clear-default"
            onClick={async () => {
              await adminApi.clearDefaultProfile();
              message.success('已清除默认 Profile');
              await refresh();
            }}
          >
            清除默认
          </Button>
        ) : null,
        <Button
          key="create"
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => openEditor()}
        >
          新建 Profile
        </Button>,
      ]}
    >
      <ProTable<AgentProfile>
        rowKey="id"
        columns={columns}
        dataSource={profiles}
        loading={profilesQuery.isLoading}
        cardBordered
        search={{ labelWidth: 'auto' }}
        scroll={{ x: 1050 }}
        pagination={{
          defaultPageSize: 20,
          showTotal: (total) => '共 ' + total + ' 个',
        }}
        options={{
          reload: () => profilesQuery.refetch(),
          density: true,
          setting: true,
        }}
        headerTitle={
          <Space size={10} align="center">
            <span>Profile 列表</span>
            <span className="table-count">共 {profiles.length} 个</span>
          </Space>
        }
        locale={{
          emptyText: (
            <Empty
              className="table-empty"
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={
                <Space direction="vertical" size={2}>
                  <Typography.Text strong>还没有 Agent Profile</Typography.Text>
                  <Typography.Text type="secondary">
                    Profile 定义 Agent 的启动参数与运行策略，先创建一个开始使用
                  </Typography.Text>
                </Space>
              }
            >
              <Button
                type="primary"
                icon={<PlusOutlined />}
                onClick={() => openEditor()}
              >
                新建 Profile
              </Button>
            </Empty>
          ),
        }}
      />

      <DrawerForm<ProfileFormValues>
        title={editing ? '编辑 Profile' : '新建 Profile'}
        width={720}
        open={open}
        onOpenChange={setOpen}
        form={form}
        drawerProps={{ destroyOnClose: true }}
        submitter={{
          searchConfig: {
            submitText: editing ? '保存修改' : '创建 Profile',
          },
          submitButtonProps: { loading: saveMutation.isPending },
        }}
        onFinish={async (values) => {
          await saveMutation.mutateAsync(values);
          return true;
        }}
      >
        <Tabs
          className="profile-form-tabs"
          items={[
            {
              key: 'basic',
              label: '基础信息',
              children: (
                <ProFormGroup>
                  <ProFormText
                    name="id"
                    label="Profile ID"
                    width="md"
                    disabled={Boolean(editing)}
                    rules={[
                      { required: true, message: '请输入 Profile ID' },
                      {
                        pattern: /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/,
                        message: '仅支持字母、数字、点、下划线和连字符',
                      },
                    ]}
                  />
                  <ProFormText
                    name="name"
                    label="名称"
                    width="md"
                    rules={[{ required: true, message: '请输入名称' }]}
                  />
                  <ProFormSelect
                    name="agent_type"
                    label="Agent 类型"
                    width="md"
                    showSearch
                    disabled={Boolean(editing)}
                    options={agentTypes.map((value) => ({
                      label: value,
                      value,
                    }))}
                    rules={[{ required: true, message: '请选择 Agent 类型' }]}
                  />
                  <ProFormSwitch name="enabled" label="启用" width="md" />
                </ProFormGroup>
              ),
            },
            {
              key: 'runtime',
              label: '运行参数',
              children: (
                <>
                  <ProFormGroup>
                    <ProFormText name="command" label="命令覆盖" width="md" />
                    <ProFormSelect
                      name="args"
                      label="启动参数"
                      width="md"
                      mode="tags"
                      options={[]}
                    />
                    <ProFormText
                      name="default_model"
                      label="默认模型"
                      width="md"
                    />
                    <ProFormText
                      name="reasoning_effort"
                      label="推理强度"
                      width="md"
                      tooltip="控制 Agent 推理深度，具体取值取决于 Agent 类型"
                    />
                    <ProFormSelect
                      name="workdir_strategy"
                      label="工作目录策略"
                      width="md"
                      options={[
                        { label: '系统默认', value: 'system_default' },
                        { label: 'Profile 默认', value: 'profile_default' },
                        {
                          label: '会话临时目录',
                          value: 'ephemeral_per_session',
                        },
                      ]}
                    />
                    <ProFormDependency name={['workdir_strategy']}>
                      {({ workdir_strategy }) =>
                        workdir_strategy === 'profile_default' ? (
                          <ProFormText
                            name="working_dir"
                            label="默认工作目录"
                            width="md"
                            rules={[
                              { required: true, message: '请输入默认工作目录' },
                            ]}
                          />
                        ) : null
                      }
                    </ProFormDependency>
                    <ProFormDigit
                      name="timeout_secs"
                      label="超时（秒）"
                      width="md"
                      min={1}
                      fieldProps={{ precision: 0 }}
                    />
                    <ProFormSelect
                      name="recovery_strategy"
                      label="恢复策略"
                      width="md"
                      options={[
                        { label: '不恢复', value: 'none' },
                        { label: '重启进程', value: 'restart_process' },
                        { label: '恢复会话', value: 'resume_session' },
                      ]}
                    />
                    <ProFormSelect
                      name="inherit_env"
                      label="继承环境变量"
                      width="md"
                      mode="tags"
                      options={[]}
                    />
                  </ProFormGroup>
                  <DynamicConfigFields agentType={agentType} />
                </>
              ),
            },
            {
              key: 'env',
              label: '环境变量',
              children: (
                <ProFormList
                  name="env_ref_entries"
                  label="环境变量引用"
                  creatorButtonProps={{
                    creatorButtonText: '添加环境变量引用',
                  }}
                >
                  <ProFormGroup>
                    <ProFormText name="key" label="变量名" width="sm" />
                    <ProFormText name="value" label="引用" width="md" />
                  </ProFormGroup>
                </ProFormList>
              ),
            },
          ]}
        />
      </DrawerForm>
    </PageContainer>
  );
}
