import { describe, expect, it } from 'vitest';
import type { TranscriptEntry } from '../types';
import {
  activityEntriesFromTranscript,
  applyTranscriptEntries,
  parseTranscriptStreamEvent,
  upsertTranscriptEntry,
} from './transcript';

function entry(overrides: Partial<TranscriptEntry> = {}): TranscriptEntry {
  return {
    entry_id: 'entry-1',
    sequence: 1,
    timestamp: '2026-08-13T00:00:00.000Z',
    role: 'assistant',
    content: 'hello',
    status: 'streaming',
    ...overrides,
  };
}

describe('transcript activity adapters', () => {
  it('upserts streaming revisions using the stable entry identity', () => {
    const initial = entry({ content: 'hel' });
    const revision = entry({ sequence: 2, content: 'hello' });

    const entries = upsertTranscriptEntry([initial], revision);

    expect(entries).toEqual([revision]);
    expect(entries).toHaveLength(1);
  });

  it('keeps distinct entries while applying a mixed replay tail', () => {
    const first = entry({ entry_id: 'assistant-1', content: 'first' });
    const revised = entry({ entry_id: 'assistant-1', sequence: 3, content: 'first revised' });
    const second = entry({ entry_id: 'tool-1', sequence: 2, role: 'tool', content: 'Run tests' });

    expect(applyTranscriptEntries([first], [second, revised])).toEqual([
      revised,
      second,
    ]);
  });

  it('maps plans, thinking and raw tool payloads to renderable feed entries', () => {
    const entries = activityEntriesFromTranscript([
      entry({ entry_id: 'thinking', status: 'thinking', content: 'Inspecting state' }),
      entry({
        entry_id: 'plan',
        role: 'system',
        status: 'plan',
        content: '- [x] Restore snapshot\n- [ ] Subscribe live events',
      }),
      entry({
        entry_id: 'tool',
        role: 'tool',
        tool_call_id: 'call-1',
        content: 'Run tests',
        status: 'completed',
        tool_call: {
          toolCallId: 'call-1',
          title: 'Run tests',
          kind: 'execute',
          rawInput: { command: 'pnpm test' },
        },
        tool_result: { content: [{ type: 'text', text: 'passed' }] },
      }),
    ]);

    expect(entries[0]).toMatchObject({ type: 'thinking', text: 'Inspecting state' });
    expect(entries[1]).toMatchObject({
      type: 'plan',
      items: [
        { text: 'Restore snapshot', done: true },
        { text: 'Subscribe live events', done: false },
      ],
    });
    expect(entries[2]).toMatchObject({
      type: 'tool',
      tool: {
        key: 'call-1',
        status: 'completed',
        description: 'pnpm test',
      },
    });
  });


  it('coalesces consecutive thinking chunks into one readable block', () => {
    const entries = activityEntriesFromTranscript([
      entry({ entry_id: 't1', status: 'thinking', content: 'Inspecting' }),
      entry({ entry_id: 't2', sequence: 2, status: 'thinking', content: 'state' }),
      entry({ entry_id: 'blank', sequence: 3, status: 'thinking', content: '   ' }),
      entry({ entry_id: 'user-1', sequence: 4, role: 'user', content: 'go' }),
    ]);

    expect(entries).toHaveLength(2);
    expect(entries[0]).toMatchObject({ type: 'thinking', text: 'Inspecting state' });
    expect(entries[1]).toMatchObject({ type: 'user', text: 'go' });
  });

  it('merges live thinking revisions onto the last thinking entry', () => {
    const first = entry({ entry_id: 'think-a', status: 'thinking', content: 'Hello' });
    const second = entry({
      entry_id: 'think-b',
      sequence: 2,
      status: 'thinking',
      content: 'world',
    });

    expect(upsertTranscriptEntry([first], second)).toEqual([
      {
        ...first,
        content: 'Hello world',
        sequence: 2,
      },
    ]);
  });
  it('accepts transcript SSE events and ignores lifecycle payloads', () => {
    expect(
      parseTranscriptStreamEvent(
        JSON.stringify({
          sequence: 7,
          session_id: 'slack:T1',
          entry: entry({ entry_id: 'entry-7', sequence: 4 }),
        }),
      ),
    ).toMatchObject({ sequence: 7, session_id: 'slack:T1' });

    expect(
      parseTranscriptStreamEvent(
        JSON.stringify({
          sequence: 8,
          event: 'status_changed',
          snapshot: { session_id: 'slack:T1' },
        }),
      ),
    ).toBeNull();
  });
});
