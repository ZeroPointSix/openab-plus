import { useEffect, useMemo } from 'react';
import {
  DrawerForm,
  ProFormSelect,
  ProFormText,
} from '@ant-design/pro-components';
import { Form, Typography } from 'antd';
import { AgentProfile, CreateSessionRequest } from '../types';

interface SessionFormValues {
  profile_id: string;
  working_dir?: string;
}

interface NewSessionDrawerProps {
  open: boolean;
  profiles: AgentProfile[];
  defaultProfile?: string;
  submitting?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateSessionRequest) => Promise<unknown>;
}

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
        await onSubmit({
          profile_id: values.profile_id,
          overrides: {
            working_dir: values.working_dir?.trim() || undefined,
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
      <Typography.Paragraph type="secondary">
        v2 要求先在 Profile 中选好服务商 / 模型 / 思考级别，再启动。启动时不再提供临时覆盖。
      </Typography.Paragraph>
      <Typography.Paragraph>
        服务商：{selectedProfile?.provider || '（未设置）'}
        <br />
        模型：{selectedProfile?.default_model || '（使用 Agent 默认）'}
        <br />
        思考级别：{selectedProfile?.reasoning_effort || '（使用 Agent 默认）'}
      </Typography.Paragraph>
      <ProFormText
        name="working_dir"
        label="工作目录覆盖"
        placeholder={selectedProfile?.working_dir || '使用 Profile 默认值'}
      />
    </DrawerForm>
  );
}
