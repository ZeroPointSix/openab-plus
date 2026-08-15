/*
 * @license
 * Copyright 2025 AionUi (aionui.com)
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted from AionUi packages/desktop/src/common/chat/normalizeToolCall.ts.
 * Modified for OpenAB Plus: framework-neutral activity feed types and ACP snapshots.
 */

import type {
  ActivityToolStatus,
  FileDiffPayload,
  NormalizedToolCall,
  TerminalOutputPayload,
} from '../../types';

type RecordValue = Record<string, unknown>;

export interface ToolCallLike {
  call_id?: string;
  id?: string;
  name?: string;
  kind?: string;
  title?: string;
  status?: string;
  description?: string;
  input?: unknown;
  args?: RecordValue;
  raw_input?: RecordValue;
  rawInput?: RecordValue;
  output?: unknown;
  content?: unknown;
  duration_ms?: number;
  durationMs?: number;
  truncated?: boolean;
  _compact?: { truncated?: boolean };
}

const asRecord = (value: unknown): RecordValue | undefined =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as RecordValue)
    : undefined;

export function formatToolValue(value: unknown): string | undefined {
  if (value === undefined || value === null || value === '') return undefined;
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function normalizeToolStatus(status?: string): ActivityToolStatus {
  switch (status?.toLowerCase()) {
    case 'success':
    case 'completed':
    case 'complete':
    case 'done':
    case 'exited':
      return 'completed';
    case 'failed':
    case 'failure':
    case 'error':
      return 'error';
    case 'cancelled':
    case 'canceled':
    case 'stopped':
      return 'canceled';
    case 'executing':
    case 'confirming':
    case 'in_progress':
    case 'in-progress':
    case 'running':
      return 'running';
    default:
      return 'pending';
  }
}

export function buildParamSummary(
  kind: string | undefined,
  rawInput?: RecordValue,
): string | undefined {
  if (!rawInput) return undefined;
  const normalizedKind = kind?.toLowerCase();

  if (normalizedKind === 'read' || normalizedKind === 'edit') {
    return stringField(rawInput, 'file_path', 'path', 'file_name');
  }
  if (normalizedKind === 'write') {
    return stringField(rawInput, 'file_path', 'path');
  }
  if (normalizedKind === 'execute' || normalizedKind === 'exec') {
    return stringField(rawInput, 'command');
  }
  if (normalizedKind === 'search' || normalizedKind === 'grep') {
    const pattern = stringField(rawInput, 'pattern', 'query');
    const path = stringField(rawInput, 'path', 'glob');
    return [pattern ? `“${pattern}”` : undefined, path ? `in ${path}` : undefined]
      .filter(Boolean)
      .join(' ') || undefined;
  }
  if (normalizedKind === 'glob') {
    const pattern = stringField(rawInput, 'pattern');
    const path = stringField(rawInput, 'path');
    return [pattern, path ? `in ${path}` : undefined].filter(Boolean).join(' ') || undefined;
  }

  return stringField(rawInput, 'file_path', 'command', 'path', 'pattern', 'query', 'url');
}

function stringField(value: RecordValue, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const candidate = value[key];
    if (typeof candidate === 'string' && candidate.trim()) return candidate;
  }
  return undefined;
}

function diffFromContent(content: unknown): FileDiffPayload | undefined {
  const record = asRecord(content);
  if (!record) return undefined;
  if (
    typeof record.path === 'string' &&
    (typeof record.old_text === 'string' ||
      typeof record.new_text === 'string' ||
      typeof record.before === 'string' ||
      typeof record.after === 'string')
  ) {
    return {
      path: record.path,
      old_text:
        typeof record.old_text === 'string'
          ? record.old_text
          : typeof record.before === 'string'
            ? record.before
            : '',
      new_text:
        typeof record.new_text === 'string'
          ? record.new_text
          : typeof record.after === 'string'
            ? record.after
            : '',
    };
  }
  return undefined;
}

function diffsFromContent(content: unknown): FileDiffPayload[] {
  if (Array.isArray(content)) {
    return content.flatMap((item) => diffsFromContent(item));
  }
  const record = asRecord(content);
  if (!record) return [];
  const direct = diffFromContent(record);
  const nested = diffsFromContent(record.diff);
  const nestedMany = diffsFromContent(record.diffs);
  return [...(direct ? [direct] : []), ...nested, ...nestedMany];
}

function terminalFromContent(content: unknown): TerminalOutputPayload | undefined {
  const record = asRecord(content);
  if (!record || typeof record.command !== 'string') return undefined;
  return {
    command: record.command,
    output: typeof record.output === 'string' ? record.output : undefined,
    exit_code: typeof record.exit_code === 'number' ? record.exit_code : undefined,
    signaled: record.signaled === true,
    truncated: record.truncated === true,
  };
}

function outputFromContent(content: unknown): string | undefined {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return undefined;

  return content
    .map((item) => {
      const record = asRecord(item);
      if (!record) return '';
      if (typeof record.text === 'string') return record.text;
      const nested = asRecord(record.content);
      if (nested && typeof nested.text === 'string') return nested.text;
      if (record.type === 'diff' && typeof record.path === 'string') return `[diff] ${record.path}`;
      return '';
    })
    .filter(Boolean)
    .join('\n') || undefined;
}

function canonicalToolName(name: string): string {
  const trimmed = name.trim();
  if (!trimmed || /^tool call$/i.test(trimmed)) return '';
  if (trimmed.toLowerCase() === 'execute' || trimmed.toLowerCase() === 'exec') {
    return 'bash';
  }
  return trimmed;
}

export function normalizeToolCall(message: ToolCallLike): NormalizedToolCall | undefined {
  const rawInput = message.rawInput ?? message.raw_input ?? message.args;
  const contentRecord = asRecord(message.content);
  const terminal = terminalFromContent(contentRecord) ?? terminalFromContent(message.output);
  const diffs = [...diffsFromContent(message.content), ...diffsFromContent(message.output)];
  const kind = message.kind || message.name;
  const key = message.call_id || message.id;
  if (!key) return undefined;
  const explicitName =
    typeof message.name === 'string' ? message.name : '';
  const kindName = typeof kind === 'string' ? kind : '';
  const titleName =
    typeof message.title === 'string' ? message.title.trim() : '';
  const toolName =
    canonicalToolName(explicitName) ||
    canonicalToolName(kindName) ||
    titleName ||
    'bash';

  return {
    key,
    name: toolName,
    kind,
    status: normalizeToolStatus(message.status),
    description: message.description || buildParamSummary(kind, rawInput),
    input: formatToolValue(message.input ?? rawInput),
    output: formatToolValue(message.output) ?? outputFromContent(message.content),
    duration_ms: message.duration_ms ?? message.durationMs,
    truncated: message.truncated === true || message._compact?.truncated === true,
    diff: diffs[0],
    diffs: diffs.length ? diffs : undefined,
    terminal,
  };
}

export function normalizeAcpToolCall(message: ToolCallLike): NormalizedToolCall | undefined {
  const update = asRecord(message.content)?.update;
  const updateRecord = asRecord(update);
  if (!updateRecord) return normalizeToolCall(message);

  const rawInput =
    asRecord(updateRecord.rawInput) ??
    asRecord(updateRecord.raw_input) ??
    message.rawInput ??
    message.raw_input;
  const kind = typeof updateRecord.kind === 'string' ? updateRecord.kind : message.kind;
  const content = updateRecord.content;
  const tool = normalizeToolCall({
    ...message,
    call_id:
      typeof updateRecord.tool_call_id === 'string'
        ? updateRecord.tool_call_id
        : message.call_id,
    title:
      typeof updateRecord.title === 'string' ? updateRecord.title : message.title,
    kind,
    status: typeof updateRecord.status === 'string' ? updateRecord.status : message.status,
    rawInput,
    content,
  });
  if (!tool) return undefined;

  return {
    ...tool,
    description: tool.description || buildParamSummary(kind, rawInput) || kind,
    input: formatToolValue(rawInput),
    output: outputFromContent(content) || tool.output,
  };
}

export function normalizeToolGroup(
  tools: ToolCallLike[],
): NormalizedToolCall[] {
  return tools
    .map((tool) => normalizeToolCall(tool))
    .filter((tool): tool is NormalizedToolCall => tool !== undefined);
}
