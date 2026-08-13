import { useEffect, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import {
  Alert,
  Button,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Steps,
  Typography,
  message,
} from 'antd';
import { ApiError, adminApi } from '../lib/api';
import { agentLaunchDefinition } from '../lib/agentLaunch';
import {
  normalizeProfilePayload,
  ProfileFormValues,
} from '../lib/profile';
import { AgentConfigField, AgentProfile } from '../types';

interface NewAgentWizardProps {
  open: boolean;
  agentTypes: string[];
  onCancel: () => void;
  onCreated: (profile: AgentProfile) => void | Promise<void>;
}

const steps = ['上游类型', '模型', '思考级别', '渠道', '凭证'];

function isCredentialReference(value: string): boolean {
  const trimmed = value.trim();
  return (
    (/^\$\{[^{}]+\}$/.test(trimmed) ||
      /^(env|aws-sm|vault|gcp-sm|azure-kv|exec):\/\/.+/.test(trimmed)) &&
    !/\s/.test(trimmed)
  );
}

function DynamicConfigField({ field }: { field: AgentConfigField }) {
  const name = ['config_options', field.id];
  if (field.options?.length) {
    return (
      <Form.Item name={name} label={field.label || field.id}>
        <Select
          allowClear
          options={field.options.map((value) => ({ label: value, value }))}
        />
      </Form.Item>
    );
  }
  if (['boolean', 'bool'].includes(field.kind)) {
    return (
      <Form.Item name={name} label={field.label || field.id}>
        <Select
          allowClear
          options={[
            { label: 'true', value: 'true' },
            { label: 'false', value: 'false' },
          ]}
        />
      </Form.Item>
    );
  }
  return (
    <Form.Item name={name} label={field.label || field.id}>
      <Input />
    </Form.Item>
  );
}

export function NewAgentWizard({
  open,
  agentTypes,
  onCancel,
  onCreated,
}: NewAgentWizardProps) {
  const [form] = Form.useForm<ProfileFormValues>();
  const [step, setStep] = useState(0);
  const agentType = Form.useWatch('agent_type', form);
  const schemaQuery = useQuery({
    queryKey: ['profileSchema', agentType],
    queryFn: () => adminApi.profileSchema(agentType as string),
    enabled: open && Boolean(agentType),
  });
  const createMutation = useMutation({
    mutationFn: async () => {
      const values = await form.validateFields();
      return adminApi.createProfile(normalizeProfilePayload(values));
    },
    onSuccess: async (document) => {
      const profileId = form.getFieldValue('id')?.trim();
      const profile = document.profiles?.find((item) => item.id === profileId);
      if (!profile) {
        message.error('Profile 已保存，但未能读取新建实体');
        return;
      }
      message.success('Agent Profile 已创建');
      await onCreated(profile);
      onCancel();
    },
    onError: (error) => {
      message.error(
        error instanceof ApiError ? error.message : '创建 Agent Profile 失败',
      );
    },
  });

  useEffect(() => {
    if (!open) return;
    const initialAgentType = agentTypes[0] || 'codex';
    const launch = agentLaunchDefinition(initialAgentType);
    setStep(0);
    form.resetFields();
    form.setFieldsValue({
      id: '',
      name: '',
      agent_type: initialAgentType,
      command: launch?.command,
      args: launch?.args || [],
      enabled: true,
      workdir_strategy: 'system_default',
      recovery_strategy: 'resume_session',
      inherit_env: [],
      config_options: {},
      env_ref_entries: [],
    });
  }, [agentTypes, form, open]);

  const applyAgentLaunch = (nextAgentType: string) => {
    const launch = agentLaunchDefinition(nextAgentType);
    form.setFieldsValue({
      agent_type: nextAgentType,
      command: launch?.command,
      args: launch?.args || [],
    });
  };

  const next = async () => {
    const fields = [
      ['id', 'name', 'agent_type', 'command'],
      ['default_model'],
      ['reasoning_effort', 'config_options'],
      ['workdir_strategy', 'working_dir'],
      ['env_ref_entries'],
    ][step];
    await form.validateFields(fields);
    setStep((current) => Math.min(current + 1, steps.length - 1));
  };

  const dynamicFields = (schemaQuery.data?.fields || []).filter(
    (field) =>
      field.dynamic && !['model', 'reasoning_effort'].includes(field.id),
  );

  const content = [
    <Space key="upstream" direction="vertical" size={8} className="agent-wizard-step">
      <Typography.Paragraph type="secondary">
        选择 ACP 上游类型并定义这个 Profile 的稳定标识。Agent Profile 是配置入口，后续会话会继承它的启动参数与策略。
      </Typography.Paragraph>
      <Form.Item
        name="id"
        label="Profile ID"
        rules={[
          { required: true, message: '请输入 Profile ID' },
          {
            pattern: /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/,
            message: '仅支持字母、数字、点、下划线和连字符',
          },
        ]}
      >
        <Input autoComplete="off" placeholder="例如 claude-research" />
      </Form.Item>
      <Form.Item
        name="name"
        label="显示名称"
        rules={[{ required: true, message: '请输入显示名称' }]}
      >
        <Input autoComplete="off" placeholder="例如 Claude 研究助手" />
      </Form.Item>
      <Form.Item
        name="agent_type"
        label="上游类型"
        rules={[{ required: true, message: '请选择 Agent 类型' }]}
      >
        <Select
          showSearch
          options={agentTypes.map((value) => ({ label: value, value }))}
          onChange={applyAgentLaunch}
        />
      </Form.Item>
      <Form.Item
        name="command"
        label="ACP 启动命令"
        rules={[{ required: true, message: '请选择上游后确认启动命令' }]}
        extra="已按镜像中的官方 ACP adapter 预填。未知上游必须明确填写可执行的 ACP 命令。"
      >
        <Input autoComplete="off" placeholder="例如 claude-agent-acp" />
      </Form.Item>
      <Form.List name="args">
        {(fields, { add, remove }) => (
          <Space direction="vertical" size={4} className="agent-wizard-command-args">
            <Typography.Text type="secondary">启动参数</Typography.Text>
            {fields.map((field) => (
              <Space key={field.key} align="start">
                <Form.Item name={field.name} rules={[{ required: true, message: '参数不能为空' }]}>
                  <Input autoComplete="off" placeholder="例如 --acp" />
                </Form.Item>
                <Button type="link" danger onClick={() => remove(field.name)}>
                  删除
                </Button>
              </Space>
            ))}
            <Button onClick={() => add()} block>
              添加启动参数
            </Button>
          </Space>
        )}
      </Form.List>
    </Space>,
    <Space key="model" direction="vertical" size={8} className="agent-wizard-step">
      <Typography.Paragraph type="secondary">
        模型会作为新会话启动前的 Profile 默认项。留空时由 Agent 或系统默认配置决定。
      </Typography.Paragraph>
      <Form.Item name="default_model" label="默认模型">
        <Input autoComplete="off" placeholder="例如 gpt-5" />
      </Form.Item>
    </Space>,
    <Space key="reasoning" direction="vertical" size={8} className="agent-wizard-step">
      <Typography.Paragraph type="secondary">
        思考级别和可选能力通过已有 live config-schema 写入 Profile，创建时不会修改任何运行中的会话。
      </Typography.Paragraph>
      <Form.Item name="reasoning_effort" label="思考级别">
        <Input autoComplete="off" placeholder="例如 low、medium 或 high" />
      </Form.Item>
      {schemaQuery.isLoading ? <Typography.Text type="secondary">正在加载 Agent 配置项…</Typography.Text> : null}
      {dynamicFields.map((field) => (
        <DynamicConfigField field={field} key={field.id} />
      ))}
    </Space>,
    <Space key="channel" direction="vertical" size={8} className="agent-wizard-step">
      <Alert
        type="info"
        showIcon
        message="渠道不绑定到 Profile"
        description="本向导不会在渠道层复制一套配置。创建后可在工作台从该 Profile 直接启动一个 Admin 来源的新会话；Discord、Slack 等渠道仍由既有部署配置和 ChatAdapter 负责。"
      />
      <Form.Item name="workdir_strategy" label="工作目录策略">
        <Select
          options={[
            { label: '系统默认', value: 'system_default' },
            { label: 'Profile 默认', value: 'profile_default' },
            { label: '会话临时目录', value: 'ephemeral_per_session' },
          ]}
        />
      </Form.Item>
      <Form.Item noStyle shouldUpdate={(previous, current) => previous.workdir_strategy !== current.workdir_strategy}>
        {({ getFieldValue }) =>
          getFieldValue('workdir_strategy') === 'profile_default' ? (
            <Form.Item
              name="working_dir"
              label="默认工作目录"
              rules={[{ required: true, message: '请输入默认工作目录' }]}
            >
              <Input autoComplete="off" placeholder="例如 /workspace" />
            </Form.Item>
          ) : null
        }
      </Form.Item>
    </Space>,
    <Space key="credentials" direction="vertical" size={8} className="agent-wizard-step">
      <Alert
        type="warning"
        showIcon
        message="仅接受凭证引用，不接受明文密钥"
        description="值会经现有 Profile env_refs 通道保存，例如 ${OPENAI_API_KEY} 或 vault://path/key。创建后页面不会回显凭证内容。"
      />
      <Form.List name="env_ref_entries">
        {(fields, { add, remove }) => (
          <Space direction="vertical" size={8} className="agent-wizard-credentials">
            {fields.map((field) => (
              <Space align="start" key={field.key}>
                <Form.Item name={[field.name, 'key']} rules={[{ required: true, message: '请输入变量名' }]}>
                  <Input placeholder="OPENAI_API_KEY" autoComplete="off" />
                </Form.Item>
                <Form.Item
                  name={[field.name, 'value']}
                  rules={[
                    { required: true, message: '请输入凭证引用' },
                    {
                      validator: (_, value) =>
                        !value || isCredentialReference(value)
                          ? Promise.resolve()
                          : Promise.reject(new Error('请输入 ${NAME}、env:// 或密钥服务引用')),
                    },
                  ]}
                >
                  <Input placeholder="${OPENAI_API_KEY}" autoComplete="off" />
                </Form.Item>
                <Button type="link" danger onClick={() => remove(field.name)}>
                  删除
                </Button>
              </Space>
            ))}
            <Button onClick={() => add()} block>
              添加凭证引用
            </Button>
          </Space>
        )}
      </Form.List>
    </Space>,
  ][step];

  return (
    <Modal
      title="New Agent"
      open={open}
      onCancel={onCancel}
      destroyOnClose
      width={680}
      footer={
        <Space>
          <Button onClick={onCancel}>取消</Button>
          {step > 0 ? <Button onClick={() => setStep((current) => current - 1)}>上一步</Button> : null}
          {step < steps.length - 1 ? (
            <Button type="primary" onClick={() => void next()}>
              下一步
            </Button>
          ) : (
            <Button type="primary" loading={createMutation.isPending} onClick={() => createMutation.mutate()}>
              创建 Agent
            </Button>
          )}
        </Space>
      }
    >
      <Form form={form} layout="vertical" preserve>
        <Steps current={step} size="small" items={steps.map((title) => ({ title }))} />
        <div className="agent-wizard-content">{content}</div>
      </Form>
    </Modal>
  );
}
