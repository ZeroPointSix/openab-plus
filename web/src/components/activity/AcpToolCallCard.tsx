/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpToolCall.tsx.
 * Modified for OpenAB Plus: read-only Ant Design activity feed with normalized tool snapshots.
 */

import {
  CheckCircleFilled,
  ClockCircleOutlined,
  CloseCircleFilled,
  LoadingOutlined,
  StopFilled,
  ToolOutlined,
} from '@ant-design/icons';
import { Card, Collapse, Tag, Typography } from 'antd';
import type { NormalizedToolCall } from '../../types';
import { FileDiff } from './FileDiff';
import { TerminalOutput } from './TerminalOutput';

function statusMeta(status: NormalizedToolCall['status']) {
  switch (status) {
    case 'completed':
      return { color: 'success' as const, label: '成功', icon: <CheckCircleFilled /> };
    case 'error':
      return { color: 'error' as const, label: '失败', icon: <CloseCircleFilled /> };
    case 'canceled':
      return { color: 'warning' as const, label: '已取消', icon: <StopFilled /> };
    case 'running':
      return { color: 'processing' as const, label: '运行中', icon: <LoadingOutlined spin /> };
    default:
      return { color: 'default' as const, label: '等待中', icon: <ClockCircleOutlined /> };
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
  const summary = tool.description || tool.kind || '无参数摘要';
  const duration = durationText(tool.duration_ms);
  const hasDetails = Boolean(tool.input || tool.output || tool.diff || tool.terminal);

  return (
    <Card size="small" className={`activity-tool-card ${tool.status}`}>
      <div className="activity-tool-summary">
        <ToolOutlined className="activity-tool-icon" aria-hidden="true" />
        <div className="activity-tool-title">
          <Typography.Text strong>{tool.name}</Typography.Text>
          <Typography.Text type="secondary" ellipsis={{ tooltip: summary }}>
            {summary}
          </Typography.Text>
        </div>
        {duration ? <Typography.Text type="secondary">{duration}</Typography.Text> : null}
        <Tag icon={status.icon} color={status.color}>
          {status.label}
        </Tag>
      </div>

      {hasDetails ? (
        <Collapse
          ghost
          className="activity-tool-details"
          items={[
            {
              key: 'details',
              label: '查看参数与结果',
              children: (
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
                    <Typography.Text type="warning">结果已截断，仅显示安全预览。</Typography.Text>
                  ) : null}
                  {tool.terminal ? <TerminalOutput terminal={tool.terminal} /> : null}
                  {tool.diff ? <FileDiff diff={tool.diff} /> : null}
                </div>
              ),
            },
          ]}
        />
      ) : null}
    </Card>
  );
}
