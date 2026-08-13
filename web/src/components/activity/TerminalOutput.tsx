/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpTerminalOutput.tsx.
 * Modified for OpenAB Plus: browser-only, read-only terminal output rendering.
 */

import { CheckCircleFilled, LoadingOutlined, StopFilled } from '@ant-design/icons';
import { Tag, Typography } from 'antd';
import { useEffect, useRef } from 'react';
import type { TerminalOutputPayload } from '../../types';

function terminalState(terminal: TerminalOutputPayload) {
  if (terminal.exit_code === undefined && !terminal.signaled) {
    return {
      icon: <LoadingOutlined spin />,
      label: '运行中',
      color: 'processing' as const,
    };
  }
  if (terminal.signaled) {
    return { icon: <StopFilled />, label: '已停止', color: 'warning' as const };
  }
  if (terminal.exit_code === 0) {
    return { icon: <CheckCircleFilled />, label: '成功', color: 'success' as const };
  }
  return {
    icon: <StopFilled />,
    label: `退出码 ${terminal.exit_code ?? '?'}`,
    color: 'error' as const,
  };
}

export function TerminalOutput({ terminal }: { terminal: TerminalOutputPayload }) {
  const outputRef = useRef<HTMLPreElement>(null);
  const state = terminalState(terminal);
  const running = terminal.exit_code === undefined && !terminal.signaled;

  useEffect(() => {
    if (running && outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [running, terminal.output]);

  return (
    <section className="activity-terminal" aria-label="终端输出">
      <div className="activity-terminal-heading">
        <Typography.Text code className="activity-terminal-command" ellipsis={{ tooltip: terminal.command }}>
          $ {terminal.command}
        </Typography.Text>
        <Tag icon={state.icon} color={state.color}>
          {state.label}
        </Tag>
      </div>
      {terminal.output || running ? (
        <pre ref={outputRef} className="activity-terminal-output">
          {terminal.truncated ? '…（更早的输出已截断）\n' : ''}
          {terminal.output || (running ? '等待终端输出…' : '')}
        </pre>
      ) : null}
    </section>
  );
}
