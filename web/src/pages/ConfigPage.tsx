import { useEffect, useMemo, useState } from 'react';
import {
  CheckCircleOutlined,
  CloudSyncOutlined,
  CodeOutlined,
  EyeInvisibleOutlined,
  FileProtectOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
  SearchOutlined,
} from '@ant-design/icons';
import { PageContainer } from '@ant-design/pro-components';
import { useQuery } from '@tanstack/react-query';
import {
  Alert,
  Button,
  Collapse,
  Descriptions,
  Empty,
  Form,
  Input,
  InputNumber,
  Select,
  Space,
  Spin,
  Switch,
  Tag,
  Typography,
  message,
} from 'antd';
import { adminApi, ApiError } from '../lib/api';
import {
  areConfigValuesEqual,
  buildConfigFields,
  cloneConfigValues,
  configValuesFrom,
  ConfigEditorField,
  maskConfigSecrets,
  parseJsonValue,
  updateConfigValue,
} from '../lib/config';
import {
  confirmNavigation,
  discardUnsavedChanges,
  setNavigationDirty,
} from '../lib/navigationGuard';
import {
  ConfigMetadata,
  ConfigValidationResult,
  ConfigValue,
  ConfigValues,
} from '../types';
import { formatDateTime } from '../lib/format';

const policyLabels = {
  runtime: { label: '实时生效', color: 'green' },
  new_session: { label: '新会话生效', color: 'blue' },
  restart_required: { label: '重启生效', color: 'orange' },
} as const;

type Operation = 'idle' | 'refreshing' | 'validating' | 'saving' | 'reloading';
type FeedbackType = 'success' | 'info' | 'warning' | 'error';

interface Feedback {
  type: FeedbackType;
  title: string;
  description: string;
  retryReload?: boolean;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof ApiError || error instanceof Error) return error.message;
  return fallback;
}

function validationSummary(result: ConfigValidationResult): string {
  return result.errors
    .slice(0, 4)
    .map((error) => error.path + '：' + error.message)
    .join('；');
}

function JsonConfigInput({
  value,
  disabled,
  onChange,
}: {
  value?: ConfigValue;
  disabled: boolean;
  onChange: (value: ConfigValue) => void;
}) {
  const formatted = value === undefined ? '' : JSON.stringify(value, null, 2);
  const [raw, setRaw] = useState(formatted);
  const [error, setError] = useState('');

  useEffect(() => {
    setRaw(formatted);
    setError('');
  }, [formatted]);

  return (
    <Space direction="vertical" size={4} className="config-json-control">
      <Input.TextArea
        value={raw}
        disabled={disabled}
        autoSize={{ minRows: 3, maxRows: 8 }}
        status={error ? 'error' : undefined}
        onChange={(event) => {
          const next = event.target.value;
          setRaw(next);
          try {
            onChange(parseJsonValue(next));
            setError('');
          } catch {
            setError('请输入有效的 JSON。');
          }
        }}
      />
      {error ? <Typography.Text type="danger">{error}</Typography.Text> : null}
    </Space>
  );
}

function ConfigFieldControl({
  field,
  disabled,
  onChange,
}: {
  field: ConfigEditorField;
  disabled: boolean;
  onChange: (value: ConfigValue) => void;
}) {
  if (field.kind === 'boolean') {
    return (
      <div className="config-switch-control">
        <Switch
          checked={field.value === true}
          disabled={disabled}
          checkedChildren="开启"
          unCheckedChildren="关闭"
          onChange={onChange}
        />
        <Typography.Text type="secondary">
          {field.value === undefined ? '未设置，沿用系统默认值' : '已显式设置'}
        </Typography.Text>
      </div>
    );
  }

  if (field.kind === 'number') {
    return (
      <InputNumber
        value={typeof field.value === 'number' ? field.value : undefined}
        disabled={disabled}
        controls
        onChange={(value) => {
          if (value !== null) onChange(value);
        }}
      />
    );
  }

  if (field.kind === 'string_array') {
    return (
      <Select
        mode="tags"
        value={
          Array.isArray(field.value)
            ? field.value.filter((item): item is string => typeof item === 'string')
            : []
        }
        disabled={disabled}
        tokenSeparators={[',']}
        placeholder="输入后按回车添加"
        onChange={onChange}
      />
    );
  }

  if (field.kind === 'json') {
    return (
      <JsonConfigInput value={field.value} disabled={disabled} onChange={onChange} />
    );
  }

  if (field.secret) {
    return (
      <Input.Password
        value={typeof field.value === 'string' ? field.value : ''}
        disabled={disabled}
        visibilityToggle={false}
        prefix={<EyeInvisibleOutlined />}
        placeholder="未设置；输入新值后保存"
        autoComplete="new-password"
        onChange={(event) => onChange(event.target.value)}
      />
    );
  }

  return (
    <Input
      value={typeof field.value === 'string' ? field.value : ''}
      disabled={disabled}
      allowClear
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

export function ConfigPage() {
  const [draft, setDraft] = useState<ConfigValues>({});
  const [baseline, setBaseline] = useState<ConfigValues>({});
  const [initialized, setInitialized] = useState(false);
  const [search, setSearch] = useState('');
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [feedback, setFeedback] = useState<Feedback>();
  const [operation, setOperation] = useState<Operation>('idle');

  const configQuery = useQuery({
    queryKey: ['config'],
    queryFn: adminApi.config,
  });
  const statusQuery = useQuery({
    queryKey: ['configStatus'],
    queryFn: adminApi.configStatus,
  });

  const metadata = configQuery.data?.metadata || [];
  const fields = useMemo(
    () => buildConfigFields(draft, metadata),
    [draft, metadata],
  );
  const dirty = initialized && !areConfigValuesEqual(draft, baseline);
  const busy = operation !== 'idle';

  const applyDocument = (values: unknown) => {
    const next = cloneConfigValues(configValuesFrom(values));
    setDraft(next);
    setBaseline(cloneConfigValues(next));
    setFieldErrors({});
    setInitialized(true);
    discardUnsavedChanges();
  };

  useEffect(() => {
    if (configQuery.data && !initialized) applyDocument(configQuery.data.values);
  }, [configQuery.data, initialized]);

  useEffect(() => {
    setNavigationDirty(dirty);
    return () => setNavigationDirty(false);
  }, [dirty]);

  useEffect(() => {
    if (!dirty) return;
    const beforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    const historyNavigation = () => {
      if (!confirmNavigation()) window.history.forward();
    };
    window.addEventListener('beforeunload', beforeUnload);
    window.addEventListener('popstate', historyNavigation);
    return () => {
      window.removeEventListener('beforeunload', beforeUnload);
      window.removeEventListener('popstate', historyNavigation);
    };
  }, [dirty]);

  const filteredGroups = useMemo(() => {
    const query = search.trim().toLowerCase();
    const grouped = new Map<string, ConfigEditorField[]>();
    for (const field of fields) {
      if (
        query &&
        !field.path.toLowerCase().includes(query) &&
        !field.label.toLowerCase().includes(query) &&
        !field.sectionLabel.toLowerCase().includes(query)
      ) {
        continue;
      }
      const group = grouped.get(field.section) || [];
      group.push(field);
      grouped.set(field.section, group);
    }
    return [...grouped.entries()].map(([key, groupFields]) => ({
      key,
      label: groupFields[0]?.sectionLabel || key,
      fields: groupFields,
    }));
  }, [fields, search]);

  const policyCounts = fields.reduce<Record<ConfigMetadata['apply_policy'], number>>(
    (counts, field) => {
      counts[field.applyPolicy] += 1;
      return counts;
    },
    { runtime: 0, new_session: 0, restart_required: 0 },
  );

  const applyValidation = (result: ConfigValidationResult) => {
    setFieldErrors(
      Object.fromEntries(result.errors.map((error) => [error.path, error.message])),
    );
  };

  const updateField = (path: string, value: ConfigValue) => {
    setDraft((current) => updateConfigValue(current, path, value));
    setFieldErrors((current) => {
      const next = { ...current };
      delete next[path];
      return next;
    });
    setFeedback(undefined);
  };

  const refresh = async () => {
    if (dirty && !window.confirm('刷新会丢弃当前未保存的修改，确定继续吗？')) return;
    setOperation('refreshing');
    setFeedback(undefined);
    try {
      const [document] = await Promise.all([
        configQuery.refetch(),
        statusQuery.refetch(),
      ]);
      if (document.data) applyDocument(document.data.values);
    } finally {
      setOperation('idle');
    }
  };

  const validateDraft = async () => {
    setOperation('validating');
    try {
      const result = await adminApi.validateConfig(cloneConfigValues(draft));
      applyValidation(result);
      if (result.ok) {
        setFeedback({
          type: 'success',
          title: '配置校验通过',
          description: '当前修改可以安全保存，配置文件尚未写入。',
        });
      } else {
        setFeedback({
          type: 'error',
          title: '配置校验未通过',
          description: validationSummary(result) || '请修正标记字段后重试。',
        });
      }
    } catch (error) {
      setFeedback({
        type: 'error',
        title: '无法完成配置校验',
        description: errorMessage(error, '校验请求失败，请稍后重试。'),
      });
    } finally {
      setOperation('idle');
    }
  };

  const reloadOnly = async () => {
    setOperation('reloading');
    try {
      const result = await adminApi.reloadConfig();
      applyValidation(result.validation);
      await statusQuery.refetch();
      if (!result.validation.ok) {
        setFeedback({
          type: 'error',
          title: '重载前校验未通过',
          description: validationSummary(result.validation),
        });
        return;
      }
      setFeedback({
        type: 'success',
        title: 'Gateway 已重新加载配置',
        description:
          '已即时应用 ' +
          result.runtime.applied_paths.length +
          ' 个字段；仍需重启 ' +
          result.status.pending_restart.length +
          ' 个字段。',
      });
      message.success('配置重载完成');
    } catch (error) {
      setFeedback({
        type: 'warning',
        title: '配置仍未完成重载',
        description: errorMessage(error, '当前进程继续使用上一次运行时配置。'),
        retryReload: true,
      });
    } finally {
      setOperation('idle');
    }
  };

  const saveAndReload = async () => {
    const values = cloneConfigValues(draft);
    let saved = false;
    setOperation('saving');
    setFeedback(undefined);
    try {
      const saveResult = await adminApi.saveConfig(values);
      applyValidation(saveResult.validation);
      if (!saveResult.validation.ok) {
        setFeedback({
          type: 'error',
          title: '配置未保存',
          description:
            validationSummary(saveResult.validation) ||
            '后端拒绝了当前修改，原配置文件保持不变。',
        });
        return;
      }

      saved = true;
      applyDocument(maskConfigSecrets(values, metadata));
      setOperation('reloading');
      const reloadResult = await adminApi.reloadConfig();
      applyValidation(reloadResult.validation);

      const [document] = await Promise.all([
        configQuery.refetch(),
        statusQuery.refetch(),
      ]);
      if (document.data) applyDocument(document.data.values);

      if (!reloadResult.validation.ok) {
        setFeedback({
          type: 'warning',
          title: '配置已保存，但没有完成动态重载',
          description:
            validationSummary(reloadResult.validation) +
            '。当前进程继续使用上一次运行时配置；可修正后重新保存。',
          retryReload: true,
        });
        return;
      }

      setFeedback({
        type: reloadResult.status.pending_restart.length ? 'info' : 'success',
        title: '配置已保存并完成动态重载',
        description:
          '已即时应用 ' +
          reloadResult.runtime.applied_paths.length +
          ' 个字段；' +
          (reloadResult.status.pending_restart.length
            ? reloadResult.status.pending_restart.length +
              ' 个字段将在 Gateway 重启后生效。'
            : '没有等待重启的字段。'),
      });
      message.success('Gateway 配置已更新');
    } catch (error) {
      if (saved) {
        setFeedback({
          type: 'warning',
          title: '配置已保存，但 reload 请求失败',
          description:
            errorMessage(error, '当前进程继续使用上一次运行时配置。') +
            (statusQuery.data?.rollback_available
              ? ' 服务器已保留上一份配置快照。'
              : ''),
          retryReload: true,
        });
        void Promise.all([configQuery.refetch(), statusQuery.refetch()]);
      } else {
        setFeedback({
          type: 'error',
          title: '配置保存失败',
          description:
            errorMessage(error, '配置文件未确认写入，请保留当前修改并重试。'),
        });
      }
    } finally {
      setOperation('idle');
    }
  };

  const status = statusQuery.data;

  return (
    <PageContainer
      title="Gateway 配置"
      subTitle="按当前配置结构编辑、校验并动态重载"
      className="page-container"
      extra={[
        <Button
          key="refresh"
          icon={<ReloadOutlined />}
          onClick={refresh}
          loading={operation === 'refreshing'}
          disabled={busy && operation !== 'refreshing'}
        >
          刷新
        </Button>,
        <Button
          key="validate"
          icon={<CheckCircleOutlined />}
          onClick={validateDraft}
          loading={operation === 'validating'}
          disabled={!initialized || (busy && operation !== 'validating')}
        >
          校验
        </Button>,
        <Button
          key="save"
          type="primary"
          icon={<CloudSyncOutlined />}
          onClick={saveAndReload}
          loading={operation === 'saving' || operation === 'reloading'}
          disabled={!dirty || busy}
        >
          保存并重载
        </Button>,
      ]}
    >
      {configQuery.isError ? (
        <Alert
          type="error"
          showIcon
          message="无法读取 Gateway 配置"
          description={errorMessage(configQuery.error, '请检查 Gateway API 后重试。')}
          action={<Button onClick={refresh}>重试</Button>}
          className="config-alert"
        />
      ) : null}
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
          message="最近一次服务端校验未通过"
          description={validationSummary(status.last_validation)}
          className="config-alert"
        />
      ) : null}
      {feedback ? (
        <Alert
          type={feedback.type}
          showIcon
          closable
          message={feedback.title}
          description={feedback.description}
          action={
            feedback.retryReload ? (
              <Button
                size="small"
                icon={<CloudSyncOutlined />}
                onClick={reloadOnly}
                loading={operation === 'reloading'}
              >
                重试重载
              </Button>
            ) : undefined
          }
          onClose={() => setFeedback(undefined)}
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
          <Space wrap>
            {dirty ? <Tag color="gold">有未保存修改</Tag> : <Tag color="green">已同步</Tag>}
            <Tag color={status?.rollback_available ? 'blue' : 'default'}>
              {status?.rollback_available ? '有回滚快照' : '无回滚快照'}
            </Tag>
          </Space>
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
        <div className="config-policy-summary">
          {(Object.keys(policyLabels) as ConfigMetadata['apply_policy'][]).map(
            (policy) => (
              <div className="config-policy-item" key={policy}>
                <Tag color={policyLabels[policy].color}>{policyLabels[policy].label}</Tag>
                <Typography.Text strong>{policyCounts[policy]}</Typography.Text>
                <Typography.Text type="secondary">个字段</Typography.Text>
              </div>
            ),
          )}
        </div>
      </section>

      <section className="config-editor-shell">
        <div className="config-editor-toolbar">
          <div className="panel-heading-title">
            <span className="panel-icon dark" aria-hidden="true">
              <CodeOutlined />
            </span>
            <div>
              <Typography.Title level={4}>配置编辑器</Typography.Title>
              <Typography.Text type="secondary">
                Secret 字段仅显示脱敏占位；保存时未修改的 Secret 会保留原值。
              </Typography.Text>
            </div>
          </div>
          <Input
            className="config-search"
            prefix={<SearchOutlined />}
            value={search}
            allowClear
            placeholder="搜索字段或平台"
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>

        {!initialized && configQuery.isLoading ? (
          <div className="config-loading">
            <Spin tip="正在读取配置" />
          </div>
        ) : filteredGroups.length ? (
          <Collapse
            className="config-sections"
            defaultActiveKey={filteredGroups.slice(0, 2).map((group) => group.key)}
            items={filteredGroups.map((group) => ({
              key: group.key,
              label: (
                <Space>
                  <Typography.Text strong>{group.label}</Typography.Text>
                  <Tag>{group.fields.length} 项</Tag>
                </Space>
              ),
              children: (
                <div className="config-field-list">
                  {group.fields.map((field) => (
                    <div className="config-field-row" key={field.path}>
                      <div className="config-field-copy">
                        <Space size={6} wrap>
                          <Typography.Text strong>{field.label}</Typography.Text>
                          <Tag color={policyLabels[field.applyPolicy].color}>
                            {policyLabels[field.applyPolicy].label}
                          </Tag>
                          {field.secret ? (
                            <Tag color="red" icon={<SafetyCertificateOutlined />}>
                              Secret
                            </Tag>
                          ) : null}
                        </Space>
                        <Typography.Text code type="secondary">
                          {field.path}
                        </Typography.Text>
                      </div>
                      <Form.Item
                        validateStatus={fieldErrors[field.path] ? 'error' : undefined}
                        help={fieldErrors[field.path]}
                      >
                        <ConfigFieldControl
                          field={field}
                          disabled={busy}
                          onChange={(value) => updateField(field.path, value)}
                        />
                      </Form.Item>
                    </div>
                  ))}
                </div>
              ),
            }))}
          />
        ) : (
          <Empty description={search ? '没有匹配的配置字段' : '当前没有可编辑字段'} />
        )}
      </section>
    </PageContainer>
  );
}
