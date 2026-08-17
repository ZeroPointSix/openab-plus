import { useEffect, useMemo } from 'react';
import {
  DrawerForm,
  ProFormGroup,
  ProFormList,
  ProFormSelect,
  ProFormText,
} from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
import { Form } from 'antd';
import { adminApi } from '../lib/api';
import { AgentProfile, CreateSessionRequest } from '../types';

interface SessionFormValues {
  profile_id: string;
  working_dir?: string;
  model?: string;
  reasoning_effort?: string;
  config_entries?: Array<{ key?: string; value?: string }>;
}

interface NewSessionDrawerProps {
  open: boolean;
  profiles: AgentProfile[];
  defaultProfile?: string;
  submitting?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateSessionRequest) => Promise<unknown>;
}

const THINKING_LEVELS = [
  'off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max',
];

export function NewSessionDrawer({
  open,
  profiles,
  defaultProfile,
  submitting,
  onOpenChange,
  onSubmit,
}: NewSessionDrawerProps) {
  const [form] = Form.useForm<SessionFormValues>();
  const profileId = Form.useWatch('profile_id', form);
  const enabledProfiles = useMemo(
    () => profiles.filter((profile) => profile.enabled),
    [profiles],
  );
  const selectedProfile = enabledProfiles.find(
    (profile) => profile.id === profileId,
  );
  const schemaQuery = useQuery({
    queryKey: ['profileSchema', selectedProfile?.agent_type],
    queryFn: () => adminApi.profileSchema(selectedProfile?.agent_type as string),
    enabled: Boolean(selectedProfile?.agent_type),
  });
  const fields = schemaQuery.data?.fields || [];
  const modelOptions = useMemo(
    () =>
      Array.from(
        new Set([
          ...(fields.find((field) => field.id === 'model')?.options || []),
          ...(selectedProfile?.default_model ? [selectedProfile.default_model] : []),
        ]),
      ).map((value) => ({ label: value, value })),
    [fields, selectedProfile?.default_model],
  );
  const reasoningOptions = useMemo(() => {
    const supported =
      fields.find((field) => field.id === 'reasoning_effort')?.options || [];
    const values = supported.length ? supported : THINKING_LEVELS;
    return Array.from(
      new Set([
        ...values,
        ...(selectedProfile?.reasoning_effort
          ? [selectedProfile.reasoning_effort]
          : []),
      ]),
    ).map((value) => ({ label: value, value }));
  }, [fields, selectedProfile?.reasoning_effort]);

  useEffect(() => {
    if (!open) return;
    const initialProfile =
      enabledProfiles.find((profile) => profile.id === defaultProfile) ||
      enabledProfiles[0];
    form.resetFields();
    form.setFieldsValue({ profile_id: initialProfile?.id });
  }, [defaultProfile, enabledProfiles, form, open]);

  return (
    <DrawerForm<SessionFormValues>
      title="新建会话"
      width={620}
      open={open}
      form={form}
      onOpenChange={onOpenChange}
      drawerProps={{ destroyOnClose: true }}
      submitter={{
        searchConfig: { submitText: '启动会话' },
        submitButtonProps: { loading: submitting },
      }}
      onFinish={async (values) => {
        const configOptions = Object.fromEntries(
          (values.config_entries || [])
            .filter((entry) => entry.key?.trim() && entry.value?.trim())
            .map((entry) => [entry.key!.trim(), entry.value!.trim()]),
        );
        await onSubmit({
          profile_id: values.profile_id,
          overrides: {
            working_dir: values.working_dir?.trim() || undefined,
            model: values.model || undefined,
            reasoning_effort: values.reasoning_effort || undefined,
            config_options:
              Object.keys(configOptions).length > 0 ? configOptions : undefined,
          },
        });
        return true;
      }}
    >
      <ProFormSelect
        name="profile_id"
        label="Profile"
        options={enabledProfiles.map((profile) => ({
          label: profile.name + ' · ' + profile.agent_type,
          value: profile.id,
        }))}
        rules={[{ required: true, message: '请选择 Profile' }]}
        showSearch
      />
      <ProFormGroup>
        <ProFormSelect
          name="model"
          label="模型覆盖"
          width="md"
          options={modelOptions}
          placeholder={selectedProfile?.default_model || '使用 Profile 默认值'}
          allowClear
          showSearch
        />
        <ProFormSelect
          name="reasoning_effort"
          label="思考级别覆盖"
          width="md"
          options={reasoningOptions}
          placeholder={selectedProfile?.reasoning_effort || '使用 Profile 默认值'}
          allowClear
        />
      </ProFormGroup>
      <ProFormText
        name="working_dir"
        label="工作目录覆盖"
        placeholder={selectedProfile?.working_dir || '使用 Profile 默认值'}
      />
      <ProFormList
        name="config_entries"
        label="启动配置覆盖"
        creatorButtonProps={{ creatorButtonText: '添加配置项' }}
      >
        <ProFormGroup key="config-entry">
          <ProFormText name="key" label="配置键" width="md" />
          <ProFormText name="value" label="配置值" width="md" />
        </ProFormGroup>
      </ProFormList>
    </DrawerForm>
  );
}
