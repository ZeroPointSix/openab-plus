import { LoadingOutlined } from '@ant-design/icons';
import { Tooltip, Typography } from 'antd';
import {
  formatDateTime,
  formatRelativeTime,
  sessionStatusDisplay,
} from '../../lib/format';
import { SessionSnapshot } from '../../types';

interface SessionRunIndicatorProps {
  session: SessionSnapshot;
}

/**
 * Compact run indicator for the workbench header (ZER-715 P0-3).
 *
 * While the agent is executing we show a small spinner instead of a heavy
 * status tag. Once it stops we show how long ago it last ran, which is the
 * information that actually matters for a stopped session.
 *
 * Note: `sessionStatusDisplay[...].active` is true for `idle` as well, so the
 * spinner is driven by `running` instead.
 */
export function SessionRunIndicator({ session }: SessionRunIndicatorProps) {
  const display =
    sessionStatusDisplay[session.status] || sessionStatusDisplay.unknown;
  const lastRunAt = session.updated_at || session.created_at;

  if (display.running) {
    return (
      <Tooltip title={display.label}>
        <span className="session-run-indicator is-running" role="status">
          <LoadingOutlined spin aria-hidden="true" />
          <span className="session-run-indicator-label">{display.label}</span>
        </span>
      </Tooltip>
    );
  }

  const relative = formatRelativeTime(lastRunAt);
  const label = relative === '-' ? display.label : '上次运行 ' + relative;

  return (
    <Tooltip title={formatDateTime(lastRunAt)}>
      <span
        className={
          'session-run-indicator' + (display.failed ? ' is-failed' : '')
        }
        role="status"
      >
        <span className="session-run-indicator-dot" aria-hidden="true" />
        <Typography.Text
          type="secondary"
          className="session-run-indicator-label"
        >
          {label}
        </Typography.Text>
      </span>
    </Tooltip>
  );
}
