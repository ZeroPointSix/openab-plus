import { AgentProfile } from '../types';

export interface KeyValueEntry {
  key?: string;
  value?: string;
}

export type ProfileFormValues = Omit<
  Partial<AgentProfile>,
  'config_options'
> & {
  env_ref_entries?: KeyValueEntry[];
  make_default?: boolean;
  config_options?: Record<string, string | number | boolean>;
};

function compactRecord(entries?: KeyValueEntry[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const entry of entries || []) {
    const key = entry.key?.trim();
    const value = entry.value?.trim();
    if (key && value) record[key] = value;
  }
  return record;
}

function compactStrings(record?: Record<string, unknown>): Record<string, string> {
  const result: Record<string, string> = {};
  for (const [key, value] of Object.entries(record || {})) {
    if (value === undefined || value === null || value === '') continue;
    result[key] = String(value);
  }
  return result;
}

export function profileToForm(profile?: AgentProfile): ProfileFormValues {
  if (!profile) {
    return {
      enabled: true,
      workdir_strategy: 'system_default',
      recovery_strategy: 'resume_session',
      args: [],
      inherit_env: [],
      config_options: {},
      env_ref_entries: [],
    };
  }
  return {
    ...profile,
    args: profile.args || [],
    inherit_env: profile.inherit_env || [],
    config_options: profile.config_options || {},
    env_ref_entries: Object.entries(profile.env_refs || {}).map(
      ([key, value]) => ({ key, value }),
    ),
  };
}

export function normalizeProfilePayload(
  values: ProfileFormValues,
): AgentProfile {
  const text = (value?: string) => value?.trim() || undefined;
  const timeout = Number(values.timeout_secs);
  return {
    id: values.id?.trim() || '',
    name: values.name?.trim() || '',
    agent_type: values.agent_type?.trim() || '',
    enabled: values.enabled !== false,
    command: text(values.command),
    args: (values.args || []).map((value) => value.trim()).filter(Boolean),
    default_model: text(values.default_model),
    reasoning_effort: text(values.reasoning_effort),
    workdir_strategy: values.workdir_strategy || 'system_default',
    working_dir: text(values.working_dir),
    env_refs: compactRecord(values.env_ref_entries),
    inherit_env: (values.inherit_env || [])
      .map((value) => value.trim())
      .filter(Boolean),
    timeout_secs: Number.isFinite(timeout) && timeout > 0 ? timeout : undefined,
    recovery_strategy: values.recovery_strategy || 'resume_session',
    config_options: compactStrings(
      values.config_options,
    ),
  };
}
