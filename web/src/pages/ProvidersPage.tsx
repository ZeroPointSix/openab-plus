import { DeleteOutlined, EditOutlined, PlusOutlined } from '@ant-design/icons';
import {
  DrawerForm,
  PageContainer,
  ProColumns,
  ProFormSelect,
  ProFormText,
  ProTable,
} from '@ant-design/pro-components';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, Form, Popconfirm, Space, Tag, message } from 'antd';
import { useEffect, useMemo, useState } from 'react';
import { adminApi, ApiError } from '../lib/api';
import { Provider } from '../types';

const PROVIDER_TYPES = [
  { label: 'OpenAI Compatible', value: 'openai_compatible' },
  { label: 'Anthropic', value: 'anthropic' },
  { label: 'Anthropic Compatible', value: 'anthropic_compatible' },
];

function isEnvOnlyRef(value?: string) {
  const trimmed = value?.trim() || '';
  return (
    (trimmed.startsWith('${') && trimmed.endsWith('}')) ||
    trimmed.startsWith('env://')
  );
}

export function ProvidersPage() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<Provider | null>(null);
  const [form] = Form.useForm<Provider>();

  const providersQuery = useQuery({
    queryKey: ['providers'],
    queryFn: async () => {
      const result = await adminApi.providers();
      if (Array.isArray((result as { providers?: Provider[] }).providers)) {
        return (result as { providers: Provider[] }).providers;
      }
      return ((result as ProviderDocumentLike).providers || []) as Provider[];
    },
  });

  const saveMutation = useMutation({
    mutationFn: async (values: Provider) => {
      const payload: Provider = {
        id: values.id.trim(),
        name: values.name.trim(),
        provider_type: values.provider_type.trim(),
        base_url: values.base_url?.trim() || undefined,
        api_key_ref: values.api_key_ref.trim(),
      };
      if (!isEnvOnlyRef(payload.api_key_ref)) {
        throw new Error('密钥引用只允许 ${VAR} 或 env://VAR');
      }
      return editing
        ? adminApi.updateProvider(editing.id, payload)
        : adminApi.createProvider(payload);
    },
    onSuccess: () => {
      message.success(editing ? '服务商已更新' : '服务商已创建');
      setOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ['providers'] });
    },
    onError: (error: unknown) => {
      message.error(
        error instanceof ApiError || error instanceof Error
          ? error.message
          : '保存失败',
      );
    },
  });

  useEffect(() => {
    if (!open) return;
    form.resetFields();
    if (editing) {
      form.setFieldsValue(editing);
    } else {
      form.setFieldsValue({
        provider_type: 'openai_compatible',
        api_key_ref: 'env://OPENAI_API_KEY',
      });
    }
  }, [editing, form, open]);

  const columns: ProColumns<Provider>[] = useMemo(
    () => [
      { title: 'ID', dataIndex: 'id', copyable: true },
      { title: '名称', dataIndex: 'name' },
      {
        title: '类型',
        dataIndex: 'provider_type',
        render: (_, row) => <Tag>{row.provider_type}</Tag>,
      },
      {
        title: 'Base URL',
        dataIndex: 'base_url',
        ellipsis: true,
        render: (_, row) => row.base_url || '-',
      },
      {
        title: '密钥引用',
        dataIndex: 'api_key_ref',
        ellipsis: true,
      },
      {
        title: '操作',
        valueType: 'option',
        render: (_, row) => [
          <Button
            key="edit"
            type="link"
            icon={<EditOutlined />}
            onClick={() => {
              setEditing(row);
              setOpen(true);
            }}
          >
            编辑
          </Button>,
          <Popconfirm
            key="delete"
            title="确认删除该服务商？"
            onConfirm={async () => {
              try {
                await adminApi.deleteProvider(row.id);
                message.success('服务商已删除');
                queryClient.invalidateQueries({ queryKey: ['providers'] });
              } catch (error) {
                message.error(
                  error instanceof ApiError ? error.message : '删除失败',
                );
              }
            }}
          >
            <Button type="link" danger icon={<DeleteOutlined />}>
              删除
            </Button>
          </Popconfirm>,
        ],
      },
    ],
    [queryClient],
  );

  return (
    <PageContainer
      title="服务商"
      subTitle="配置模型上游。密钥只保存环境变量引用，不会回显明文。"
      extra={
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setEditing(null);
            setOpen(true);
          }}
        >
          新建服务商
        </Button>
      }
    >
      <ProTable<Provider>
        rowKey="id"
        search={false}
        options={false}
        loading={providersQuery.isLoading}
        dataSource={providersQuery.data || []}
        columns={columns}
        pagination={{ pageSize: 10 }}
      />
      <DrawerForm<Provider>
        title={editing ? '编辑服务商' : '新建服务商'}
        open={open}
        form={form}
        drawerProps={{ destroyOnClose: true }}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) setEditing(null);
        }}
        onFinish={async (values) => {
          await saveMutation.mutateAsync(values);
          return true;
        }}
      >
        <ProFormText
          name="id"
          label="服务商 ID"
          disabled={Boolean(editing)}
          rules={[{ required: true, message: '请输入 ID' }]}
        />
        <ProFormText
          name="name"
          label="名称"
          rules={[{ required: true, message: '请输入名称' }]}
        />
        <ProFormSelect
          name="provider_type"
          label="类型"
          options={PROVIDER_TYPES}
          rules={[{ required: true, message: '请选择类型' }]}
        />
        <ProFormText name="base_url" label="Base URL" placeholder="https://..." />
        <ProFormText
          name="api_key_ref"
          label="密钥引用"
          tooltip="仅支持 ${VAR} 或 env://VAR"
          rules={[
            { required: true, message: '请输入密钥引用' },
            {
              validator: async (_, value) => {
                if (!isEnvOnlyRef(value)) {
                  throw new Error('只允许 ${VAR} 或 env://VAR');
                }
              },
            },
          ]}
        />
        <Space direction="vertical" size={4}>
          <Tag color="blue">密钥值永不回显</Tag>
          <Tag>示例：env://OPENAI_API_KEY 或 ${'{'}ANTHROPIC_API_KEY{'}'}</Tag>
        </Space>
      </DrawerForm>
    </PageContainer>
  );
}

type ProviderDocumentLike = { providers?: Provider[] };
