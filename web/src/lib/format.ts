/**
 * 平台标识 → 展示名称。用于「回到 Slack / Discord」等来源跳转文案。
 */
export function sourcePlatformLabel(platform?: string): string {
  if (!platform) return '';
  const labels: Record<string, string> = {
    slack: 'Slack',
    discord: 'Discord',
    acp: 'ACP',
    telegram: 'Telegram',
  };
  return labels[platform] || platform;
}

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

export function eventLabel(event: string): string {
  const labels: Record<string, string> = {
    'session.created': '会话创建',
    status_changed: '状态变更',
    model_changed: '模型变更',
    profile_changed: 'Profile 变更',
    profile_deleted: 'Profile 已删除',
    session_error: '会话异常',
    current: '当前状态',
  };
  return labels[event] || event.replaceAll('_', ' ');
}
