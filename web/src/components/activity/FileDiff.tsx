/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpToolCall.tsx.
 * Modified for OpenAB Plus: browser-only, read-only bounded hunk preview.
 */

import { FileTextOutlined } from '@ant-design/icons';
import { Collapse, Empty, Typography } from 'antd';
import { useMemo } from 'react';
import type { FileDiffPayload } from '../../types';

export type DiffLineKind = 'context' | 'added' | 'removed' | 'omitted';

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  oldNumber?: number;
  newNumber?: number;
}

const DEFAULT_CONTEXT_LINES = 3;
const MAX_CHANGED_LINES = 240;

function splitLines(value: string): string[] {
  if (!value) return [];
  const lines = value.split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

function pairedContext(
  before: string[],
  after: string[],
  oldStart: number,
  newStart: number,
  count: number,
): DiffLine[] {
  return before.slice(oldStart, oldStart + count).map((text, index) => ({
    kind: 'context',
    text,
    oldNumber: oldStart + index + 1,
    newNumber: newStart + index + 1,
  }));
}

function boundedChanges(lines: DiffLine[]): DiffLine[] {
  if (lines.length <= MAX_CHANGED_LINES) return lines;
  const firstCount = Math.ceil(MAX_CHANGED_LINES / 2);
  const lastCount = Math.floor(MAX_CHANGED_LINES / 2);
  const omitted = lines.length - firstCount - lastCount;
  return [
    ...lines.slice(0, firstCount),
    { kind: 'omitted', text: `… ${omitted} changed lines omitted from preview` },
    ...lines.slice(-lastCount),
  ];
}

/**
 * Builds a bounded unified hunk around the changed region. This intentionally
 * avoids a full O(n×m) LCS matrix: a transcript diff snapshot already carries
 * both sides, so preserving common prefix/suffix context is sufficient for a
 * readable, safe preview and always retains the changed lines.
 */
export function buildHunkPreview(
  oldText: string,
  newText: string,
  context = DEFAULT_CONTEXT_LINES,
): DiffLine[] {
  const before = splitLines(oldText);
  const after = splitLines(newText);
  let prefix = 0;
  const sharedLimit = Math.min(before.length, after.length);
  while (prefix < sharedLimit && before[prefix] === after[prefix]) prefix += 1;

  let suffix = 0;
  while (
    suffix < before.length - prefix &&
    suffix < after.length - prefix &&
    before[before.length - 1 - suffix] === after[after.length - 1 - suffix]
  ) {
    suffix += 1;
  }

  if (prefix === before.length && prefix === after.length) {
    return pairedContext(before, after, 0, 0, before.length);
  }

  const oldChangedEnd = before.length - suffix;
  const newChangedEnd = after.length - suffix;
  const leadingStart = Math.max(0, prefix - context);
  const leadingCount = prefix - leadingStart;
  const trailingCount = Math.min(context, suffix);
  const lines: DiffLine[] = [];

  if (leadingStart > 0) {
    lines.push({ kind: 'omitted', text: `… ${leadingStart} unchanged lines omitted` });
  }
  lines.push(...pairedContext(before, after, leadingStart, leadingStart, leadingCount));

  const changed = [
    ...before.slice(prefix, oldChangedEnd).map((text, index) => ({
      kind: 'removed' as const,
      text,
      oldNumber: prefix + index + 1,
    })),
    ...after.slice(prefix, newChangedEnd).map((text, index) => ({
      kind: 'added' as const,
      text,
      newNumber: prefix + index + 1,
    })),
  ];
  lines.push(...boundedChanges(changed));

  if (trailingCount) {
    lines.push(
      ...pairedContext(
        before,
        after,
        oldChangedEnd,
        newChangedEnd,
        trailingCount,
      ),
    );
  }
  if (suffix > trailingCount) {
    lines.push({
      kind: 'omitted',
      text: `… ${suffix - trailingCount} unchanged lines omitted`,
    });
  }
  return lines;
}

export function FileDiff({ diff }: { diff: FileDiffPayload }) {
  const lines = useMemo(
    () => buildHunkPreview(diff.old_text, diff.new_text),
    [diff.new_text, diff.old_text],
  );
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
          children: lines.length ? (
            <div className="activity-diff-shell" role="region" aria-label={`${diff.path} diff`}>
              {lines.map((line, index) => (
                <div className={`activity-diff-line ${line.kind}`} key={`${line.kind}-${index}-${line.text}`}>
                  <span className="activity-diff-number">{line.oldNumber || ''}</span>
                  <span className="activity-diff-number">{line.newNumber || ''}</span>
                  <span className="activity-diff-marker" aria-hidden="true">
                    {line.kind === 'added' ? '+' : line.kind === 'removed' ? '−' : line.kind === 'omitted' ? '⋮' : ' '}
                  </span>
                  <code>{line.text || ' '}</code>
                </div>
              ))}
            </div>
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="没有可显示的差异" />
          ),
        },
      ]}
    />
  );
}
