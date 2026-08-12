import {
  formatToolValue,
  normalizeAcpToolCall,
  normalizeToolStatus,
} from '../components/activity/normalizeToolCall';
import type {
  ActivityEntry,
  FileDiffPayload,
  NormalizedToolCall,
  TranscriptEntry,
  TranscriptEvent,
  TranscriptSnapshot,
} from '../types';

type RecordValue = Record<string, unknown>;

function asRecord(value: unknown): RecordValue | undefined {
  return typeof value === 'object' &&
    value !== null &&
    !Array.isArray(value)
    ? (value as RecordValue)
    : undefined;
}

function firstString(
  value: RecordValue,
  ...keys: string[]
): string | undefined {
  for (const key of keys) {
    const candidate = value[key];
    if (typeof candidate === 'string' && candidate.trim()) {
      return candidate;
    }
  }
  return undefined;
}

function transcriptDiffs(payload: RecordValue): FileDiffPayload[] {
  const candidates = [
    payload.diff,
    ...(Array.isArray(payload.diffs) ? payload.diffs : []),
  ];

  return candidates.flatMap((candidate) => {
    const diff = asRecord(candidate);
    if (!diff || typeof diff.path !== 'string') return [];
    const oldText =
      typeof diff.old_text === 'string'
        ? diff.old_text
        : typeof diff.before === 'string'
          ? diff.before
          : undefined;
    const newText =
      typeof diff.new_text === 'string'
        ? diff.new_text
        : typeof diff.after === 'string'
          ? diff.after
          : undefined;
    if (oldText === undefined && newText === undefined) return [];
    return [
      {
        path: diff.path,
        old_text: oldText || '',
        new_text: newText || '',
      },
    ];
  });
}

function toolFromTranscript(entry: TranscriptEntry): NormalizedToolCall {
  const payload = {
    ...(asRecord(entry.tool_call) || {}),
    ...(asRecord(entry.tool_result) || {}),
  };
  const callId =
    entry.tool_call_id ||
    firstString(payload, 'toolCallId', 'tool_call_id') ||
    entry.entry_id;
  const title =
    entry.content || firstString(payload, 'title', 'name') || 'Tool call';
  const kind = firstString(payload, 'kind', 'name', 'sessionUpdate');
  const rawInput =
    asRecord(payload.rawInput) ||
    asRecord(payload.raw_input) ||
    asRecord(payload.args);
  const tool = normalizeAcpToolCall({
    call_id: callId,
    title,
    kind,
    status: entry.status,
    content: {
      update: {
        ...payload,
        tool_call_id: callId,
        title,
        status: entry.status,
        rawInput,
      },
    },
  }) || {
    key: callId,
    name: title,
    kind,
    status: normalizeToolStatus(entry.status),
  };
  const diffs = tool.diffs?.length
    ? tool.diffs
    : transcriptDiffs(payload);

  return {
    ...tool,
    output: tool.output || formatToolValue(entry.tool_result),
    diff: diffs[0],
    diffs: diffs.length ? diffs : undefined,
  };
}

function planItems(content: string): Array<{ text: string; done?: boolean }> {
  return content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const done = /^\s*[-*]?\s*\[[xX]\]\s+/.test(line);
      const text = line
        .replace(/^\s*[-*]?\s*\[[ xX]\]\s+/, '')
        .replace(/^\s*(?:[-*]|\d+[.)])\s+/, '');
      return { text, done: done || undefined };
    });
}

function activityFromTranscript(
  entry: TranscriptEntry,
): ActivityEntry | undefined {
  const content = entry.content || '';
  const base = { id: entry.entry_id, created_at: entry.timestamp };
  const status = entry.status?.toLowerCase();

  if (entry.role === 'tool') {
    return { ...base, type: 'tool', tool: toolFromTranscript(entry) };
  }
  if (!content) return undefined;
  if (entry.role === 'user') {
    return { ...base, type: 'user', text: content };
  }
  if (entry.role === 'assistant' && status === 'thinking') {
    return { ...base, type: 'thinking', text: content };
  }
  if (entry.role === 'system' && status === 'plan') {
    return {
      ...base,
      type: 'plan',
      title: '执行计划',
      items: planItems(content),
    };
  }
  if (
    entry.role === 'system' &&
    (status === 'error' || status === 'failed')
  ) {
    return { ...base, type: 'error', message: content };
  }
  return { ...base, type: 'assistant', text: content };
}

export function transcriptEntriesToActivity(
  entries: TranscriptEntry[],
): ActivityEntry[] {
  const activity: ActivityEntry[] = [];
  let turn = 0;

  for (const entry of entries) {
    if (entry.role === 'user') {
      turn += 1;
      activity.push({
        id: 'turn-' + entry.entry_id,
        type: 'turn',
        label: '回合 ' + turn,
        created_at: entry.timestamp,
      });
    }
    const item = activityFromTranscript(entry);
    if (item) activity.push(item);
  }

  return activity;
}

export function parseTranscriptEventPayload(
  data: string,
): TranscriptEvent | undefined {
  try {
    const value = asRecord(JSON.parse(data));
    const entry = asRecord(value?.entry);
    if (
      !value ||
      !entry ||
      typeof value.sequence !== 'number' ||
      typeof value.session_id !== 'string' ||
      typeof entry.entry_id !== 'string' ||
      typeof entry.sequence !== 'number'
    ) {
      return undefined;
    }
    return value as unknown as TranscriptEvent;
  } catch {
    return undefined;
  }
}

export function upsertTranscriptEntry(
  current: TranscriptSnapshot | undefined,
  event: TranscriptEvent,
): TranscriptSnapshot {
  if (!current || current.session_id !== event.session_id) {
    return {
      session_id: event.session_id,
      entries: [event.entry],
      overflowed: false,
      oldest_sequence: event.entry.sequence,
      next_sequence: event.entry.sequence + 1,
    };
  }

  const entries = [...current.entries];
  const index = entries.findIndex(
    (entry) => entry.entry_id === event.entry.entry_id,
  );
  if (index >= 0) {
    if (entries[index].sequence > event.entry.sequence) return current;
    entries[index] = event.entry;
  } else {
    entries.push(event.entry);
  }

  return {
    ...current,
    entries,
    next_sequence: Math.max(
      current.next_sequence,
      event.entry.sequence + 1,
    ),
  };
}
