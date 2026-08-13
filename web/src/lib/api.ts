import {
  AgentConfigSchema,
  AgentProfile,
  AgentProfileDocument,
  AgentSummary,
  CreateSessionRequest,
  ConfigDocument,
  ConfigReloadResponse,
  ConfigStatus,
  ConfigUpdateResponse,
  ConfigValidationResult,
  ConfigValues,
  ProfileValidationResult,
  SessionSnapshot,
} from '../types';
import { notifyUnauthorized, readAdminToken } from './auth';

export class ApiError extends Error {
  readonly status: number;
  readonly payload: unknown;

  constructor(status: number, message: string, payload: unknown) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.payload = payload;
  }
}

interface ApiRequestOptions extends RequestInit {
  token?: string;
  suppressUnauthorized?: boolean;
}

async function parsePayload(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function errorMessage(payload: unknown, fallback: string): string {
  if (typeof payload === 'string' && payload.trim()) return payload;
  if (payload && typeof payload === 'object') {
    const value = payload as Record<string, unknown>;
    if (typeof value.error === 'string') return value.error;
    if (typeof value.message === 'string') return value.message;
  }
  return fallback;
}

export async function apiRequest<T>(
  path: string,
  options: ApiRequestOptions = {},
): Promise<T> {
  const {
    token: explicitToken,
    suppressUnauthorized = false,
    ...requestOptions
  } = options;
  const token = (explicitToken ?? readAdminToken()).trim();
  const headers = new Headers(requestOptions.headers);
  if (token) headers.set('Authorization', 'Bearer ' + token);
  if (requestOptions.body && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }

  const response = await fetch(path, { ...requestOptions, headers });
  const payload = await parsePayload(response);
  if (!response.ok) {
    if (response.status === 401 && !suppressUnauthorized) {
      notifyUnauthorized();
    }
    throw new ApiError(
      response.status,
      errorMessage(payload, '请求失败（HTTP ' + response.status + '）'),
      payload,
    );
  }
  return payload as T;
}

export const adminApi = {
  probe: (token: string) =>
    apiRequest<SessionSnapshot[]>('/api/v1/sessions', {
      token,
      suppressUnauthorized: true,
    }),
  sessions: () => apiRequest<SessionSnapshot[]>('/api/v1/sessions'),
  session: (id: string) =>
    apiRequest<SessionSnapshot>(
      '/api/v1/sessions/' + encodeURIComponent(id),
    ),
  createSession: (request: CreateSessionRequest) =>
    apiRequest<SessionSnapshot>('/api/v1/sessions', {
      method: 'POST',
      body: JSON.stringify(request),
    }),
  profiles: () =>
    apiRequest<AgentProfileDocument>('/api/v1/agent-profiles'),
  agents: () => apiRequest<AgentSummary[]>('/api/v1/agents'),
  profileSchema: (agentType: string) =>
    apiRequest<AgentConfigSchema>(
      '/api/v1/agents/' +
        encodeURIComponent(agentType) +
        '/config-schema',
    ),
  createProfile: (profile: AgentProfile) =>
    apiRequest<AgentProfileDocument>('/api/v1/agent-profiles', {
      method: 'POST',
      body: JSON.stringify(profile),
    }),
  updateProfile: (id: string, profile: AgentProfile) =>
    apiRequest<AgentProfileDocument>(
      '/api/v1/agent-profiles/' + encodeURIComponent(id),
      {
        method: 'PUT',
        body: JSON.stringify(profile),
      },
    ),
  deleteProfile: (id: string) =>
    apiRequest<{ deleted: boolean }>(
      '/api/v1/agent-profiles/' + encodeURIComponent(id),
      { method: 'DELETE' },
    ),
  setDefaultProfile: (id: string) =>
    apiRequest<AgentProfileDocument>(
      '/api/v1/agent-profiles/default/' + encodeURIComponent(id),
      { method: 'PUT' },
    ),
  clearDefaultProfile: () =>
    apiRequest<AgentProfileDocument>('/api/v1/agent-profiles/default', {
      method: 'DELETE',
    }),
  validateProfile: (id: string) =>
    apiRequest<ProfileValidationResult>(
      '/api/v1/agent-profiles/' +
        encodeURIComponent(id) +
        '/validate',
      { method: 'POST' },
    ),
  config: () => apiRequest<ConfigDocument>('/api/v1/config'),
  configStatus: () => apiRequest<ConfigStatus>('/api/v1/config/status'),
  validateConfig: (values: ConfigValues) =>
    apiRequest<ConfigValidationResult>('/api/v1/config/validate', {
      method: 'POST',
      body: JSON.stringify({ values }),
    }),
  saveConfig: (values: ConfigValues) =>
    apiRequest<ConfigUpdateResponse>('/api/v1/config', {
      method: 'PUT',
      body: JSON.stringify({ values }),
    }),
  reloadConfig: () =>
    apiRequest<ConfigReloadResponse>('/api/v1/config/reload', {
      method: 'POST',
    }),
};
