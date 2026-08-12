import { describe, expect, it } from 'vitest';
import type {
  TranscriptEntry,
  TranscriptEvent,
  TranscriptSnapshot,
} from '../types';
import {
  parseTranscriptEventPayload,
  transcriptEntriesToActivity,
  upsertTranscriptEntry,
} from './transcript';

const at = '2026-08-12T08:00:00.000Z';

function entry(
  value: Partial<TranscriptEntry> &
    Pick<TranscriptEntry, 'entry_id' | 'sequence' | 'role'>,
): TranscriptEntry {
  return { timestamp: at, ...value };
}

describe('transcript activity adapter', () => {
  it('maps real transcript roles, plans, and ACP tool payloads', () => {
    const activity = transcriptEntriesToActivity([
      entry({
        entry_id: 'user-1',
        sequence: 1,
        role: 'user',
        content: '检查测试',
      }),
      entry({
        entry_id: 'thinking-1',
        sequence: 2,
        role: 'assistant',
        status: 'thinking',
        content: '先读取状态',
      }),
      entry({
        entry_id: 'plan-1',
        sequence: 3,
        role: 'system',
        status: 'plan',
        content: '- [x] 读取状态\n- 执行测试',
      }),
      entry({
        entry_id: 'tool-1',
        sequence: 4,
        role: 'tool',
        content: '运行测试',
        tool_call_id: 'call-1',
        status: 'completed',
        tool_call: {
          sessionUpdate: 'tool_call',
          kind: 'execute',
          rawInput: { command: 'pnpm test' },
        },
        tool_result: {
          sessionUpdate: 'tool_call_update',
          content: [{ type: 'text', text: '30 tests passed' }],
        },
      }),
    ]);

    expect(activity.map((item) => item.type)).toEqual([
      'turn',
      'user',
      'thinking',
      'plan',
      'tool',
    ]);
    expect(activity[3]).toMatchObject({
      type: 'plan',
      items: [
        { text: '读取状态', done: true },
        { text: '执行测试' },
      ],
    });
    expect(activity[4]).toMatchObject({
      type: 'tool',
      tool: {
        key: 'call-1',
        status: 'completed',
        description: 'pnpm test',
        output: '30 tests passed',
      },
    });
  });

  it('upserts streamed revisions without duplicating stable entries', () => {
    const current: TranscriptSnapshot = {
      session_id: 'session-1',
      entries: [
        entry({
          entry_id: 'assistant-1',
          sequence: 1,
          role: 'assistant',
          content: 'hello',
        }),
      ],
      overflowed: false,
      oldest_sequence: 1,
      next_sequence: 2,
    };
    const event: TranscriptEvent = {
      sequence: 9,
      session_id: 'session-1',
      entry: entry({
        entry_id: 'assistant-1',
        sequence: 2,
        role: 'assistant',
        content: 'hello world',
      }),
    };

    expect(upsertTranscriptEntry(current, event)).toMatchObject({
      next_sequence: 3,
      entries: [{ entry_id: 'assistant-1', content: 'hello world' }],
    });
  });

  it('parses transcript SSE payloads and rejects lifecycle payloads', () => {
    const payload = JSON.stringify({
      sequence: 4,
      session_id: 'session-1',
      entry: entry({
        entry_id: 'user-1',
        sequence: 1,
        role: 'user',
        content: 'hello',
      }),
    });

    expect(parseTranscriptEventPayload(payload)?.session_id).toBe('session-1');
    expect(
      parseTranscriptEventPayload(
        JSON.stringify({ sequence: 4, event: 'status_changed' }),
      ),
    ).toBeUndefined();
  });
});
