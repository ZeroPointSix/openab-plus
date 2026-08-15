import { Empty, Typography } from 'antd';
import { formatDateTime } from '../../lib/format';
import { TranscriptEntry } from '../../types';

interface SessionEventLogProps {
  entries: TranscriptEntry[];
}

const roleLabels: Record<string, string> = {
  user: '用户',
  assistant: 'Agent',
  system: '系统',
  tool: '工具',
};

const SUMMARY_MAX_LENGTH = 160;

function summaryFor(entry: TranscriptEntry): string {
  const content = entry.content?.trim();
  if (content) {
    const flattened = content.replace(/\s*\n+\s*/g, ' ⏎ ');
    return flattened.length > SUMMARY_MAX_LENGTH
      ? flattened.slice(0, SUMMARY_MAX_LENGTH) + '…'
      : flattened;
  }
  if (entry.tool_call_id) return 'tool_call_id=' + entry.tool_call_id;
  if (entry.status) return 'status=' + entry.status;
  return '(无文本内容)';
}

function lineFor(entry: TranscriptEntry): string {
  const time = formatDateTime(entry.timestamp);
  const role = (roleLabels[entry.role] || entry.role).padEnd(6);
  return '#' + entry.sequence + '  ' + time + '  ' + role + '  ' + summaryFor(entry);
}

/**
 * Read-only execution log (ZER-715 P1-5).
 *
 * This used to render a client-only timeline that was seeded with exactly two
 * synthetic items on every page load, so a session that had run many times
 * still showed only the first two entries. It now renders the server-backed
 * transcript history instead.
 */
export function SessionEventLog({ entries }: SessionEventLogProps) {
  if (!entries.length) {
    return (
      <Empty
        image={Empty.PRESENTED_IMAGE_SIMPLE}
        description="暂无执行日志"
        className="session-event-log-empty"
      />
    );
  }

  const lines = [...entries]
    .sort((a, b) => a.sequence - b.sequence)
    .map(lineFor);

  return (
    <section className="session-event-log" aria-label="只读执行日志">
      <Typography.Text type="secondary" className="inspector-events-caption">
        只读日志流 · 服务端 transcript 全量历史，共 {entries.length} 条
      </Typography.Text>
      <pre className="session-event-log-body">{lines.join('\n')}</pre>
    </section>
  );
}
