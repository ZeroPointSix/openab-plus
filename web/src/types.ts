export type SessionStatus =
  | 'starting'
  | 'idle'
  | 'running'
  | 'suspended'
  | 'error'
  | 'exited'
  | 'unknown';

export interface SessionSource {
  platform: string;
  thread_id: string;
}

export interface ProfileConfigError {
  config_id: string;
  error: string;
}

export interface SessionSnapshot {
  session_id: string;
  agent: string;
  source: SessionSource;
  workdir: string;
  profile_id?: string;
  profile_name?: string;
  profile_status?: 'active' | 'deleted';
  model?: string;
  status: SessionStatus;
  last_error?: string;
  profile_config_errors?: ProfileConfigError[];
  created_at: string;
  updated_at: string;
  external_url?: string;
}

export interface SessionEventPayload {
  sequence: number;
  event: string;
  snapshot?: SessionSnapshot;
}

export interface SessionTimelineItem {
  id: string;
  event: string;
  status: SessionStatus;
  at: string;
  error?: string;
  sequence?: number;
}

export type WorkdirStrategy =
  | 'system_default'
  | 'profile_default'
  | 'ephemeral_per_session';

export type RecoveryStrategy =
  | 'none'
  | 'restart_process'
  | 'resume_session';

export interface AgentProfile {
  id: string;
  name: string;
  agent_type: string;
  enabled: boolean;
  command?: string;
  args?: string[];
  default_model?: string;
  reasoning_effort?: string;
  workdir_strategy: WorkdirStrategy;
  working_dir?: string;
  env_refs?: Record<string, string>;
  inherit_env?: string[];
  timeout_secs?: number;
  recovery_strategy: RecoveryStrategy;
  config_options?: Record<string, string>;
  created_at?: string;
  updated_at?: string;
}

export interface AgentProfileDocument {
  default_profile?: string;
  profiles?: AgentProfile[];
}

export interface AgentSummary {
  agent_type: string;
  profile_count: number;
  enabled_profile_count: number;
  default_profile?: string;
}

export interface AgentConfigField {
  id: string;
  key?: string;
  label: string;
  kind: string;
  type?: string;
  options?: string[];
  dynamic?: boolean;
  apply_after_start?: boolean;
}

export interface AgentConfigSchema {
  agent_type: string;
  source: string;
  generated_at: string;
  fields: AgentConfigField[];
}

export interface ProfileValidationError {
  path: string;
  code: string;
  message: string;
}

export interface ProfileValidationResult {
  ok: boolean;
  errors: ProfileValidationError[];
}

export type ConfigScalar = string | number | boolean | null;
export type ConfigValue =
  | ConfigScalar
  | ConfigValue[]
  | { [key: string]: ConfigValue };
export type ConfigValues = Record<string, ConfigValue>;

export interface ConfigMetadata {
  path: string;
  apply_policy: 'runtime' | 'new_session' | 'restart_required';
  secret: boolean;
}

export interface ConfigDocument {
  values: ConfigValues;
  metadata: ConfigMetadata[];
}

export interface ConfigValidationError {
  path: string;
  code: string;
  message: string;
}

export interface ConfigValidationResult {
  ok: boolean;
  errors: ConfigValidationError[];
}

export interface ConfigStatus {
  config_path: string;
  last_saved_at?: string;
  last_loaded_hash?: string;
  pending_restart: string[];
  rollback_available: boolean;
  last_validation?: ConfigValidationResult;
}

export interface ConfigUpdateResponse {
  validation: ConfigValidationResult;
  status: ConfigStatus;
}

export interface ConfigReloadResponse {
  validation: ConfigValidationResult;
  runtime: {
    applied_paths: string[];
  };
  status: ConfigStatus;
}

export interface SessionFilters {
  platform?: string;
  status?: string;
  profile?: string;
  updatedRange?: [string, string];
}
