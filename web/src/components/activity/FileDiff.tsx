/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpToolCall.tsx.
 * Modified for OpenAB Plus: browser-only, read-only bounded multi-hunk preview.
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

type ChangeCluster = {
  start: number;
  end: number;
};

const DEFAULT_CONTEXT_LINES = 3;
const MAX_CHANGED_LINES = 240;
const RESYNC_LOOKAHEAD = 64;

function splitLines(value: string): string[] {
  if (!value) return [];
  const lines = value.split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

function findWithin(lines: string[], value: string, start: number): number | undefined {
  const end = Math.min(lines.length, start + RESYNC_LOOKAHEAD);
  for (let index = start; index < end; index += 1) {
    if (lines[index] === value) return index;
  }
  return undefined;
}

function buildLineOperations(before: string[], after: string[]): DiffLine[] {
  const operations: DiffLine[] = [];
  let oldIndex = 0;
  let newIndex = 0;

  while (oldIndex < before.length || newIndex < after.length) {
    if (
      oldIndex < before.length &&
      newIndex < after.length &&
      before[oldIndex] === after[newIndex]
    ) {
      operations.push({
        kind: 'context',
        text: before[oldIndex],
        oldNumber: oldIndex + 1,
        newNumber: newIndex + 1,
      });
      oldIndex += 1;
      newIndex += 1;
      continue;
    }

    if (oldIndex >= before.length) {
      operations.push({
        kind: 'added',
        text: after[newIndex],
        newNumber: newIndex + 1,
      });
      newIndex += 1;
      continue;
    }

    if (newIndex >= after.length) {
      operations.push({
        kind: 'removed',
        text: before[oldIndex],
        oldNumber: oldIndex + 1,
      });
      oldIndex += 1;
      continue;
    }

    const nextNewMatch = findWithin(after, before[oldIndex], newIndex + 1);
    const nextOldMatch = findWithin(before, after[newIndex], oldIndex + 1);
    const addedDistance = nextNewMatch === undefined ? Infinity : nextNewMatch - newIndex;
    const removedDistance = nextOldMatch === undefined ? Infinity : nextOldMatch - oldIndex;

    if (addedDistance < removedDistance) {
      operations.push({
        kind: 'added',
        text: after[newIndex],
        newNumber: newIndex + 1,
      });
      newIndex += 1;
      continue;
    }

    if (removedDistance < addedDistance) {
      operations.push({
        kind: 'removed',
        text: before[oldIndex],
        oldNumber: oldIndex + 1,
      });
      oldIndex += 1;
      continue;
    }

    operations.push(
      {
        kind: 'removed',
        text: before[oldIndex],
        oldNumber: oldIndex + 1,
      },
      {
        kind: 'added',
        text: after[newIndex],
        newNumber: newIndex + 1,
      },
    );
    oldIndex += 1;
    newIndex += 1;
  }

  return operations;
}

function changeClusters(operations: DiffLine[]): ChangeCluster[] {
  const clusters: ChangeCluster[] = [];
  let start: number | undefined;

  operations.forEach((line, index) => {
    if (line.kind !== 'context') {
      start ??= index;
      return;
    }
    if (start !== undefined) {
      clusters.push({ start, end: index });
      start = undefined;
    }
  });

  if (start !== undefined) clusters.push({ start, end: operations.length });
  return clusters;
}

function boundedCluster(lines: DiffLine[], limit: number): DiffLine[] {
  if (lines.length <= limit) return lines;
  const firstCount = Math.ceil(limit / 2);
  const lastCount = Math.floor(limit / 2);
  const omitted = lines.length - firstCount - lastCount;
  return [
    ...lines.slice(0, firstCount),
    { kind: 'omitted', text: `… ${omitted} changed lines omitted from this hunk` },
    ...lines.slice(-lastCount),
  ];
}

function hunkRanges(
  operations: DiffLine[],
  clusters: ChangeCluster[],
  context: number,
): Array<ChangeCluster> {
  return clusters.map((cluster, index) => {
    let start = cluster.start;
    let end = cluster.end;

    if (index === 0) {
      start = Math.max(0, cluster.start - context);
    } else {
      const previous = clusters[index - 1];
      const gap = cluster.start - previous.end;
      start = cluster.start - Math.min(context, Math.floor(gap / 2));
    }

    if (index === clusters.length - 1) {
      end = Math.min(operations.length, cluster.end + context);
    } else {
      const next = clusters[index + 1];
      const gap = next.start - cluster.end;
      end = cluster.end + Math.min(context, Math.ceil(gap / 2));
    }

    return { start, end };
  });
}

/**
 * Builds a bounded multi-hunk preview without an O(n×m) LCS matrix. A small
 * resynchronization window handles insertions and deletions, while each sparse
 * changed region receives its own context so a distant middle edit cannot be
 * hidden by a global prefix/suffix truncation.
 */
export function buildHunkPreview(
  oldText: string,
  newText: string,
  context = DEFAULT_CONTEXT_LINES,
): DiffLine[] {
  if (oldText === newText) return [];

  const operations = buildLineOperations(splitLines(oldText), splitLines(newText));
  const clusters = changeClusters(operations);
  if (!clusters.length) return [];

  const ranges = hunkRanges(operations, clusters, context);
  const changedLinesPerHunk = Math.max(
    2,
    Math.floor(MAX_CHANGED_LINES / clusters.length),
  );
  const preview: DiffLine[] = [];
  let renderedUntil = 0;

  clusters.forEach((cluster, index) => {
    const range = ranges[index];
    if (range.start > renderedUntil) {
      preview.push({
        kind: 'omitted',
        text: `… ${range.start - renderedUntil} unchanged lines omitted`,
      });
    }

    preview.push(...operations.slice(range.start, cluster.start));
    preview.push(
      ...boundedCluster(
        operations.slice(cluster.start, cluster.end),
        changedLinesPerHunk,
      ),
    );
    preview.push(...operations.slice(cluster.end, range.end));
    renderedUntil = range.end;
  });

  if (renderedUntil < operations.length) {
    preview.push({
      kind: 'omitted',
      text: `… ${operations.length - renderedUntil} unchanged lines omitted`,
    });
  }

  return preview;
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
