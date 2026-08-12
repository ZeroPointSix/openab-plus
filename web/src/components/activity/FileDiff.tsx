/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpToolCall.tsx.
 * Modified for OpenAB Plus: browser-only, read-only Ant Design diff preview.
 */

import { FileTextOutlined } from '@ant-design/icons';
import { Collapse, Empty, Typography } from 'antd';
import { useMemo } from 'react';
import type { FileDiffPayload } from '../../types';

type DiffLineKind = 'context' | 'added' | 'removed';

interface DiffLine {
  kind: DiffLineKind;
  text: string;
  oldNumber?: number;
  newNumber?: number;
}

const MAX_DIFF_LINES = 280;

function buildLineDiff(oldText: string, newText: string): DiffLine[] {
  const before = oldText.split('\n');
  const after = newText.split('\n');
  const matrix = Array.from({ length: before.length + 1 }, () =>
    new Uint16Array(after.length + 1),
  );

  for (let oldIndex = before.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = after.length - 1; newIndex >= 0; newIndex -= 1) {
      matrix[oldIndex][newIndex] =
        before[oldIndex] === after[newIndex]
          ? matrix[oldIndex + 1][newIndex + 1] + 1
          : Math.max(matrix[oldIndex + 1][newIndex], matrix[oldIndex][newIndex + 1]);
    }
  }

  const lines: DiffLine[] = [];
  let oldIndex = 0;
  let newIndex = 0;
  while (oldIndex < before.length || newIndex < after.length) {
    if (before[oldIndex] === after[newIndex]) {
      lines.push({
        kind: 'context',
        text: before[oldIndex] || '',
        oldNumber: oldIndex + 1,
        newNumber: newIndex + 1,
      });
      oldIndex += 1;
      newIndex += 1;
    } else if (
      newIndex < after.length &&
      (oldIndex === before.length || matrix[oldIndex][newIndex + 1] >= matrix[oldIndex + 1][newIndex])
    ) {
      lines.push({ kind: 'added', text: after[newIndex], newNumber: newIndex + 1 });
      newIndex += 1;
    } else if (oldIndex < before.length) {
      lines.push({ kind: 'removed', text: before[oldIndex], oldNumber: oldIndex + 1 });
      oldIndex += 1;
    }
  }
  return lines;
}

export function FileDiff({ diff }: { diff: FileDiffPayload }) {
  const lines = useMemo(
    () => buildLineDiff(diff.old_text, diff.new_text),
    [diff.new_text, diff.old_text],
  );
  const isTruncated = lines.length > MAX_DIFF_LINES;
  const visibleLines = isTruncated ? lines.slice(0, MAX_DIFF_LINES) : lines;
  const fileName = diff.path.split('/').filter(Boolean).at(-1) || diff.path;

  return (
    <Collapse
      className="activity-file-diff"
      defaultActiveKey={['diff']}
      items={[
        {
          key: 'diff',
          label: (
            <span className="activity-file-diff-label">
              <FileTextOutlined />
              <Typography.Text ellipsis={{ tooltip: diff.path }}>{fileName}</Typography.Text>
              <Typography.Text type="secondary" className="activity-file-diff-path">
                {diff.path}
              </Typography.Text>
            </span>
          ),
          children: visibleLines.length ? (
            <div className="activity-diff-shell" role="region" aria-label={`${diff.path} diff`}>
              {visibleLines.map((line, index) => (
                <div className={`activity-diff-line ${line.kind}`} key={`${line.kind}-${index}-${line.text}`}>
                  <span className="activity-diff-number">{line.oldNumber || ''}</span>
                  <span className="activity-diff-number">{line.newNumber || ''}</span>
                  <span className="activity-diff-marker" aria-hidden="true">
                    {line.kind === 'added' ? '+' : line.kind === 'removed' ? '−' : ' '}
                  </span>
                  <code>{line.text || ' '}</code>
                </div>
              ))}
              {isTruncated ? (
                <div className="activity-diff-truncated">
                  Diff 预览仅显示前 {MAX_DIFF_LINES} 行。
                </div>
              ) : null}
            </div>
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="没有可显示的差异" />
          ),
        },
      ]}
    />
  );
}
