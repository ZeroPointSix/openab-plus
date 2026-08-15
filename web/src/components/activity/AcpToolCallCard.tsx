/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpToolCall.tsx.
 * Modified for OpenAB Plus: read-only one-line ink summary with expandable details.
 */

import {
  CheckCircleOutlined,
  ClockCircleOutlined,
  CloseCircleOutlined,
  LoadingOutlined,
  StopOutlined,
  ToolOutlined,
} from '@ant-design/icons';
import { Collapse, Typography } from 'antd';
import type { NormalizedToolCall } from '../../types';
import { FileDiff } from './FileDiff';
import { TerminalOutput } from './TerminalOutput';

function statusMeta(status: NormalizedToolCall['status']) {
  switch (status) {
    case 'completed':
      return { label: '完成', icon: <CheckCircleOutlined /> };
    case 'error':
      return { label: '失败', icon: <CloseCircleOutlined /> };
    case 'canceled':
      return { label: '已取消', icon: <StopOutlined /> };
    case 'running':
      return { label: '运行中', icon: <LoadingOutlined spin /> };
    default:
      return { label: '待执行', icon: <ClockCircleOutlined /> };
  }
}

function durationText(duration: number | undefined) {
  if (duration === undefined) return undefined;
  if (duration < 1_000) return `${duration} ms`;
  return `${(duration / 1_000).toFixed(duration < 10_000 ? 1 : 0)} s`;
}

function CodeBlock({ children }: { children: string }) {
  return <pre className="activity-code-block">{children}</pre>;
}

export function AcpToolCallCard({ tool }: { tool: NormalizedToolCall }) {
  const status = statusMeta(tool.status);
  const summary = tool.description || tool.kind || '';
  const duration = durationText(tool.duration_ms);
  const diffs = tool.diffs || (tool.diff ? [tool.diff] : []);
  const hasDetails = Boolean(tool.input || tool.output || diffs.length || tool.terminal);
  const title = tool.name || tool.kind || 'bash';

  return (
    <article className={`activity-tool-row ${tool.status}`}>
      <Collapse
        ghost
        className="activity-tool-details"
        items={[
          {
            key: 'details',
            showArrow: hasDetails,
            collapsible: hasDetails ? 'header' : 'disabled',
            label: (
              <div className="activity-tool-summary">
                <ToolOutlined className="activity-tool-icon" aria-hidden="true" />
                <div className="activity-tool-title">
                  <Typography.Text strong className="activity-tool-name">
                    {title}
                  </Typography.Text>
                  {summary ? (
                    <Typography.Text type="secondary" ellipsis={{ tooltip: summary }}>
                      {summary}
                    </Typography.Text>
                  ) : null}
                </div>
                {duration ? (
                  <Typography.Text type="secondary">{duration}</Typography.Text>
                ) : null}
                <span className={'activity-tool-status is-' + tool.status}>
                  {status.icon}
                  {status.label}
                </span>
              </div>
            ),
            children: hasDetails ? (
              <div className="activity-tool-detail-content">
                {tool.input ? (
                  <section>
                    <Typography.Text type="secondary">参数</Typography.Text>
                    <CodeBlock>{tool.input}</CodeBlock>
                  </section>
                ) : null}
                {tool.output ? (
                  <section>
                    <Typography.Text type="secondary">结果</Typography.Text>
                    <CodeBlock>{tool.output}</CodeBlock>
                  </section>
                ) : null}
                {tool.truncated ? (
                  <Typography.Text type="warning">
                    结果已截断，仅显示安全预览。
                  </Typography.Text>
                ) : null}
                {tool.terminal ? <TerminalOutput terminal={tool.terminal} /> : null}
                {diffs.map((diff, index) => (
                  <FileDiff diff={diff} key={`${diff.path}-${index}`} />
                ))}
              </div>
            ) : null,
          },
        ]}
      />
    </article>
  );
}
