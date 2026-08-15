import type {
  ActivityEntry,
  TranscriptEntry,
  TranscriptStreamEvent,
} from '../types';
import {
  normalizeAcpToolCall,
  type ToolCallLike,
} from '../components/activity/normalizeToolCall';

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : undefined;
}

function planItems(content: string): Array<{ text: string; done?: boolean }> {
  return content
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const match = line.match(
        /^(?:[-*+]\s+|\d+[.)]\s+)?(?:\[([ xX])\]\s*)?(.*)$/,
      );
      const text = (match?.[2] || line).trim();
      return {
        text,
        done: match?.[1]?.toLowerCase() === 'x',
      };
    })
    .filter((item) => item.text.length > 0);
}

export function isThinkingEntry(entry: TranscriptEntry): boolean {
  return entry.status === 'thinking';
}

export function isBlankThinking(entry: TranscriptEntry): boolean {
  return isThinkingEntry(entry) && !(entry.content || '').trim();
}

export function joinThinkingText(left: string, right: string): string {
  const a = left || '';
  const b = right || '';
  if (!a) return b;
  if (!b) return a;
  if (/\s$/.test(a) || /^\s/.test(b)) return a + b;
  if (/^[,.;:!?)]/.test(b) || /[(['"]$/.test(a)) return a + b;
  return `${a} ${b}`;
}

function toToolEntry(entry: TranscriptEntry): ActivityEntry {
  const call = asRecord(entry.tool_call) || {};
  const result = entry.tool_result;
  const normalized = normalizeAcpToolCall({
    ...call,
    call_id:
      (typeof call.toolCallId === 'string' && call.toolCallId) ||
      entry.tool_call_id ||
      entry.entry_id,
    id: entry.entry_id,
    name: typeof call.name === 'string' ? call.name : undefined,
    kind: typeof call.kind === 'string' ? call.kind : undefined,
    title:
      (typeof call.title === 'string' && call.title) ||
      entry.content ||
      undefined,
    status: entry.status,
    output: result,
  } as ToolCallLike);

  if (normalized) {
    return {
      id: entry.entry_id,
      created_at: entry.timestamp,
      type: 'tool',
      tool: normalized,
    };
  }

  return {
    id: entry.entry_id,
    created_at: entry.timestamp,
    type: 'error',
    message: entry.content || '收到无法识别的工具调用事件。',
  };
}

/** Converts the read-only transcript protocol into the activity-feed view model. */
export function activityEntryFromTranscript(
  entry: TranscriptEntry,
): ActivityEntry {
  const content = entry.content || '';
  const base = {
    id: entry.entry_id,
    created_at: entry.timestamp,
  };

  if (entry.role === 'tool' || entry.tool_call || entry.tool_call_id) {
    return toToolEntry(entry);
  }

  if (entry.status === 'thinking') {
    return { ...base, type: 'thinking', text: content };
  }

  if (entry.status === 'plan') {
    return {
      ...base,
      type: 'plan',
      title: '执行计划',
      items: planItems(content),
    };
  }

  if (entry.status === 'error') {
    return { ...base, type: 'error', message: content || 'Agent 运行异常。' };
  }

  if (entry.role === 'user') {
    return { ...base, type: 'user', text: content };
  }

  return { ...base, type: 'assistant', text: content };
}

/**
 * Applies an entry revision in place. Stable `entry_id` values let streaming
 * text and tool lifecycle updates replace their prior visible entry instead of
 * growing one row per chunk. Consecutive thinking fragments stay as separate
 * revisable rows here and are only joined in the activity-feed view model.
 */
export function upsertTranscriptEntry(
  entries: TranscriptEntry[],
  incoming: TranscriptEntry,
): TranscriptEntry[] {
  const index = entries.findIndex((entry) => entry.entry_id === incoming.entry_id);
  if (index >= 0) {
    const next = [...entries];
    next[index] = incoming;
    return next;
  }

  if (isBlankThinking(incoming)) {
    return entries;
  }

  return [...entries, incoming];
}

export function applyTranscriptEntries(
  entries: TranscriptEntry[],
  incoming: TranscriptEntry[],
): TranscriptEntry[] {
  return incoming.reduce(upsertTranscriptEntry, entries);
}

export function activityEntriesFromTranscript(
  entries: TranscriptEntry[],
): ActivityEntry[] {
  const coalesced: ActivityEntry[] = [];
  for (const raw of entries) {
    if (isBlankThinking(raw)) continue;
    const entry = activityEntryFromTranscript(raw);
    const last = coalesced[coalesced.length - 1];
    if (entry.type === 'thinking' && last?.type === 'thinking') {
      last.text = joinThinkingText(last.text, entry.text);
      last.created_at = entry.created_at;
      continue;
    }
    if (entry.type === 'thinking' && !entry.text.trim()) continue;
    coalesced.push(entry);
  }
  return coalesced;
}

function isTranscriptEntry(value: unknown): value is TranscriptEntry {
  const entry = asRecord(value);
  return Boolean(
    entry &&
      typeof entry.entry_id === 'string' &&
      Number.isInteger(entry.sequence) &&
      typeof entry.timestamp === 'string' &&
      typeof entry.role === 'string',
  );
}

/** Parses only transcript events; lifecycle events intentionally return null. */
export function parseTranscriptStreamEvent(
  data: string,
): TranscriptStreamEvent | null {
  try {
    const value = JSON.parse(data) as JsonRecord;
    if (
      !Number.isInteger(value.sequence) ||
      typeof value.session_id !== 'string' ||
      !isTranscriptEntry(value.entry)
    ) {
      return null;
    }
    return value as unknown as TranscriptStreamEvent;
  } catch {
    return null;
  }
}

export function parseStreamProblem(data: string): string | null {
  try {
    const value = JSON.parse(data) as JsonRecord;
    return typeof value.error === 'string' ? value.error : null;
  } catch {
    return null;
  }
}

export function streamLatencyMs(timestamp: string): number | undefined {
  const sentAt = Date.parse(timestamp);
  if (Number.isNaN(sentAt)) return undefined;
  return Math.max(0, Date.now() - sentAt);
}
