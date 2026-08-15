import { SessionStatus } from '../types';

type SessionStatusDisplay = {
  label: string;
  tagColor: string;
  timelineColor: string;
  active: boolean;
  running: boolean;
  failed: boolean;
};

export const sessionStatusDisplay: Record<SessionStatus, SessionStatusDisplay> = {
  starting: {
    label: '启动中',
    tagColor: 'processing',
    timelineColor: 'blue',
    active: true,
    running: true,
    failed: false,
  },
  idle: {
    label: '空闲',
    tagColor: 'default',
    timelineColor: 'gray',
    active: true,
    running: false,
    failed: false,
  },
  running: {
    label: '运行中',
    tagColor: 'success',
    timelineColor: 'green',
    active: true,
    running: true,
    failed: false,
  },
  suspended: {
    label: '已暂停',
    tagColor: 'warning',
    timelineColor: 'orange',
    active: true,
    running: false,
    failed: false,
  },
  error: {
    label: '失败',
    tagColor: 'error',
    timelineColor: 'red',
    active: false,
    running: false,
    failed: true,
  },
  exited: {
    label: '已完成',
    tagColor: 'default',
    timelineColor: 'gray',
    active: false,
    running: false,
    failed: false,
  },
  unknown: {
    label: '未知',
    tagColor: 'default',
    timelineColor: 'gray',
    active: false,
    running: false,
    failed: false,
  },
};

export const sessionStatusOptions = (
  Object.entries(sessionStatusDisplay) as Array<
    [SessionStatus, SessionStatusDisplay]
  >
).map(([value, display]) => ({ value, label: display.label }));

export function formatDateTime(value?: string): string {
  if (!value) return '-';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(date);
}

export function formatRelativeTime(value?: string): string {
  if (!value) return '-';
  const time = new Date(value).getTime();
  if (Number.isNaN(time)) return value;
  const diff = time - Date.now();
  const abs = Math.abs(diff);
  const formatter = new Intl.RelativeTimeFormat('zh-CN', { numeric: 'auto' });
  if (abs < 60_000) return formatter.format(Math.round(diff / 1000), 'second');
  if (abs < 3_600_000) return formatter.format(Math.round(diff / 60_000), 'minute');
  if (abs < 86_400_000) return formatter.format(Math.round(diff / 3_600_000), 'hour');
  return formatter.format(Math.round(diff / 86_400_000), 'day');
}

export function sourcePlatformLabel(platform?: string): string {
  const labels: Record<string, string> = {
    slack: 'Slack',
    discord: 'Discord',
    webhook: 'Webhook',
    admin: 'Admin',
  };
  return platform ? labels[platform.toLowerCase()] || platform : '';
}

export function agentDisplayName(agent?: string): string {
  if (!agent) return '未知 Agent';
  const packageName = agent.split('/').at(-1) || agent;
  return packageName
    .replace(/-agent-acp$/i, '')
    .replace(/-acp$/i, '')
    .replaceAll('-', ' ')
    .replace(/\b\w/g, (value) => value.toUpperCase());
}

export const transcriptStatusLabels = {
  loading: '正在加载历史',
  connecting: '正在连接',
  live: '已连接',
  reconnecting: '正在重连',
  offline: '离线',
  recovery_needed: '需要恢复',
} as const;

export type TranscriptStatusKey = keyof typeof transcriptStatusLabels;

export function transcriptStatusLabel(status: TranscriptStatusKey): string {
  return transcriptStatusLabels[status];
}

export function eventLabel(event: string): string {
  const labels: Record<string, string> = {
    'session.created': '会话创建',
    status_changed: '状态变更',
    model_changed: '模型变更',
    profile_changed: 'Profile 变更',
    source_changed: '来源链接已更新',
    profile_deleted: 'Profile 已删除',
    session_error: '会话异常',
    current: '当前状态',
  };
  return labels[event] || event.replaceAll('_', ' ');
}
