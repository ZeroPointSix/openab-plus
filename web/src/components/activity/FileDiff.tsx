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
const MAX_PREVIEW_HUNKS = 64;
const RESYNC_LOOKAHEAD = 64;

function splitLines(value: string): string[] {
  if (!value) return [];
  const lines = value.split('\n');
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

function findWithin(
  lines: string[],
  value: string,
  start: number,
  limit: number,
): number | undefined {
  const end = Math.min(limit, start + RESYNC_LOOKAHEAD);
  for (let index = start; index < end; index += 1) {
    if (lines[index] === value) return index;
  }
  return undefined;
}

function buildLineOperations(before: string[], after: string[]): DiffLine[] {
  const operations: DiffLine[] = [];
  let prefixLength = 0;
  while (
    prefixLength < before.length &&
    prefixLength < after.length &&
    before[prefixLength] === after[prefixLength]
  ) {
    operations.push({
      kind: 'context',
      text: before[prefixLength],
      oldNumber: prefixLength + 1,
      newNumber: prefixLength + 1,
    });
    prefixLength += 1;
  }

  let suffixLength = 0;
  while (
    suffixLength < before.length - prefixLength &&
    suffixLength < after.length - prefixLength &&
    before[before.length - suffixLength - 1] ===
      after[after.length - suffixLength - 1]
  ) {
    suffixLength += 1;
  }

  const oldEnd = before.length - suffixLength;
  const newEnd = after.length - suffixLength;
  let oldIndex = prefixLength;
  let newIndex = prefixLength;

  while (oldIndex < oldEnd || newIndex < newEnd) {
    if (
      oldIndex < oldEnd &&
      newIndex < newEnd &&
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

    if (oldIndex >= oldEnd) {
      operations.push({
        kind: 'added',
        text: after[newIndex],
        newNumber: newIndex + 1,
      });
      newIndex += 1;
      continue;
    }

    if (newIndex >= newEnd) {
      operations.push({
        kind: 'removed',
        text: before[oldIndex],
        oldNumber: oldIndex + 1,
      });
      oldIndex += 1;
      continue;
    }

    const nextNewMatch = findWithin(
      after,
      before[oldIndex],
      newIndex + 1,
      newEnd,
    );
    const nextOldMatch = findWithin(
      before,
      after[newIndex],
      oldIndex + 1,
      oldEnd,
    );
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

  for (let index = 0; index < suffixLength; index += 1) {
    const oldNumber = oldEnd + index + 1;
    const newNumber = newEnd + index + 1;
    operations.push({
      kind: 'context',
      text: before[oldNumber - 1],
      oldNumber,
      newNumber,
    });
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
    ...(lastCount ? lines.slice(-lastCount) : []),
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
  const selectedIndexes = (() => {
    if (clusters.length <= MAX_PREVIEW_HUNKS) {
      return clusters.map((_, index) => index);
    }
    const selected = new Set<number>();
    const denominator = MAX_PREVIEW_HUNKS - 1;
    for (let index = 0; index < MAX_PREVIEW_HUNKS; index += 1) {
      selected.add(Math.round((index * (clusters.length - 1)) / denominator));
    }
    return [...selected].sort((a, b) => a - b);
  })();
  const changedLinesPerHunk = Math.max(
    1,
    Math.floor(MAX_CHANGED_LINES / selectedIndexes.length),
  );
  const preview: DiffLine[] = [];
  let renderedUntil = 0;
  let previousClusterIndex = -1;

  selectedIndexes.forEach((clusterIndex) => {
    const cluster = clusters[clusterIndex];
    const range = ranges[clusterIndex];
    if (clusterIndex > previousClusterIndex + 1) {
      preview.push({
        kind: 'omitted',
        text: `… ${clusterIndex - previousClusterIndex - 1} changed hunks omitted`,
      });
    }
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
    previousClusterIndex = clusterIndex;
  });

  if (previousClusterIndex < clusters.length - 1) {
    preview.push({
      kind: 'omitted',
      text: `… ${clusters.length - previousClusterIndex - 1} changed hunks omitted`,
    });
  }
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
