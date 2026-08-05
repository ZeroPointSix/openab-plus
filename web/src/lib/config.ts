import {
  ConfigMetadata,
  ConfigValue,
  ConfigValues,
} from '../types';

export const MASKED_SECRET = '********';

export type ConfigFieldKind =
  | 'boolean'
  | 'number'
  | 'string'
  | 'string_array'
  | 'json';

export interface ConfigEditorField {
  path: string;
  key: string;
  label: string;
  section: string;
  sectionLabel: string;
  kind: ConfigFieldKind;
  value?: ConfigValue;
  applyPolicy: ConfigMetadata['apply_policy'];
  secret: boolean;
}

const SECTION_LABELS: Record<string, string> = {
  gateway: 'Gateway 核心',
  telegram: 'Telegram',
  line: 'LINE',
  wecom: '企业微信',
  googlechat: 'Google Chat',
  teams: 'Microsoft Teams',
  feishu: '飞书',
};

const FIELD_LABELS: Record<string, string> = {
  url: '连接地址',
  platform: '平台',
  webhook_path: 'Webhook 路径',
  allowed_users: '允许的用户',
  allow_all_users: '允许所有用户',
  rich_messages: '富文本消息',
  trusted_source_only: '仅信任来源',
  streaming: '流式输出',
  streaming_enabled: '启用流式输出',
  debounce_secs: '防抖时间（秒）',
  bot_token: 'Bot Token',
  secret_token: 'Secret Token',
  channel_secret: 'Channel Secret',
  channel_access_token: 'Channel Access Token',
  app_secret: 'App Secret',
  verification_token: 'Verification Token',
  encrypt_key: 'Encrypt Key',
  access_token: 'Access Token',
  sa_key_json: 'Service Account Key',
  encoding_aes_key: 'Encoding AES Key',
  token: 'Token',
  secret: 'Secret',
};

const BOOLEAN_KEYS = new Set([
  'allow_all_users',
  'enabled',
  'rich_messages',
  'streaming',
  'streaming_enabled',
  'trusted_source_only',
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function isSecretPath(path: string): boolean {
  const key = path.split('.').at(-1)?.toLowerCase() || '';
  return ['token', 'secret', 'password', 'credential', 'key'].some((part) =>
    key.includes(part),
  );
}

function inferKind(path: string, value: unknown, secret: boolean): ConfigFieldKind {
  if (secret) return 'string';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number') return 'number';
  if (Array.isArray(value)) {
    return value.every((item) => typeof item === 'string')
      ? 'string_array'
      : 'json';
  }
  if (isRecord(value)) return 'json';

  const key = path.split('.').at(-1)?.toLowerCase() || '';
  if (BOOLEAN_KEYS.has(key) || key.startsWith('enable_')) return 'boolean';
  if (
    key.endsWith('_secs') ||
    key.endsWith('_ms') ||
    key.endsWith('_minutes') ||
    key.endsWith('_hours') ||
    key === 'port'
  ) {
    return 'number';
  }
  if (key.startsWith('allowed_') || key.endsWith('_list')) return 'string_array';
  return 'string';
}

function fieldLabel(path: string): string {
  const key = path.split('.').at(-1) || path;
  return FIELD_LABELS[key] || key.replaceAll('_', ' ');
}

function collectLeafPaths(value: unknown, prefix: string, paths: Set<string>) {
  if (isRecord(value)) {
    for (const [key, child] of Object.entries(value)) {
      collectLeafPaths(child, prefix ? prefix + '.' + key : key, paths);
    }
    return;
  }
  if (prefix) paths.add(prefix);
}

export function configValuesFrom(value: unknown): ConfigValues {
  return isRecord(value) ? (value as ConfigValues) : {};
}

export function cloneConfigValues(values: ConfigValues): ConfigValues {
  return JSON.parse(JSON.stringify(values)) as ConfigValues;
}

export function editableSecretValue(value: ConfigValue | undefined): string {
  return typeof value === 'string' && value !== MASKED_SECRET ? value : '';
}

export function findUnsafeIntegerPaths(values: ConfigValues): string[] {
  const paths: string[] = [];

  const visit = (value: unknown, path: string) => {
    if (typeof value === 'number' && Number.isInteger(value) && !Number.isSafeInteger(value)) {
      paths.push(path || '<root>');
      return;
    }
    if (Array.isArray(value)) {
      value.forEach((child, index) => visit(child, path + '[' + index + ']'));
      return;
    }
    if (isRecord(value)) {
      for (const [key, child] of Object.entries(value)) {
        visit(child, path ? path + '.' + key : key);
      }
    }
  };

  visit(values, '');
  return paths;
}

export function getConfigValue(
  values: ConfigValues,
  path: string,
): ConfigValue | undefined {
  const result = path
    .split('.')
    .reduce<unknown>((current, part) =>
      isRecord(current) ? current[part] : undefined,
    values);
  return result as ConfigValue | undefined;
}

export function updateConfigValue(
  values: ConfigValues,
  path: string,
  value: ConfigValue,
): ConfigValues {
  const next = cloneConfigValues(values);
  const parts = path.split('.');
  let cursor: Record<string, ConfigValue> = next;

  for (const part of parts.slice(0, -1)) {
    const child = cursor[part];
    if (!isRecord(child)) cursor[part] = {};
    cursor = cursor[part] as Record<string, ConfigValue>;
  }
  cursor[parts.at(-1) as string] = value;
  return next;
}

export function buildConfigFields(
  values: ConfigValues,
  metadata: ConfigMetadata[],
): ConfigEditorField[] {
  const paths = new Set<string>();
  collectLeafPaths(values, '', paths);
  metadata.forEach((item) => paths.add(item.path));
  const metadataByPath = new Map(metadata.map((item) => [item.path, item]));

  return [...paths]
    .sort((left, right) => left.localeCompare(right))
    .map((path) => {
      const item = metadataByPath.get(path);
      const value = getConfigValue(values, path);
      const secret = item?.secret || isSecretPath(path);
      const section = path.split('.')[0] || 'gateway';
      return {
        path,
        key: path.split('.').at(-1) || path,
        label: fieldLabel(path),
        section,
        sectionLabel: SECTION_LABELS[section] || section.replaceAll('_', ' '),
        kind: inferKind(path, value, secret),
        value,
        applyPolicy: item?.apply_policy || 'restart_required',
        secret,
      };
    });
}

function canonicalize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (!isRecord(value)) return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, canonicalize(value[key])]),
  );
}

export function areConfigValuesEqual(
  left: ConfigValues,
  right: ConfigValues,
): boolean {
  return JSON.stringify(canonicalize(left)) === JSON.stringify(canonicalize(right));
}

export function maskConfigSecrets(
  values: ConfigValues,
  metadata: ConfigMetadata[],
): ConfigValues {
  const explicitSecrets = new Set(
    metadata.filter((item) => item.secret).map((item) => item.path),
  );

  const visit = (value: unknown, path: string): unknown => {
    if (Array.isArray(value)) {
      return value.map((child, index) => visit(child, path + '.' + index));
    }
    if (!isRecord(value)) return value;

    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => {
        const childPath = path ? path + '.' + key : key;
        if (explicitSecrets.has(childPath) || isSecretPath(childPath)) {
          return [key, MASKED_SECRET];
        }
        return [key, visit(child, childPath)];
      }),
    );
  };

  return visit(values, '') as ConfigValues;
}

export function parseJsonValue(value: string): ConfigValue {
  return JSON.parse(value) as ConfigValue;
}
