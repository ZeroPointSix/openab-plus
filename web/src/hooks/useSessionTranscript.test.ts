import { createElement } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { TranscriptEntry, TranscriptSnapshot } from '../types';
import { adminApi } from '../lib/api';
import { saveAdminToken } from '../lib/auth';
import {
  useSessionTranscript,
  type TranscriptRecoveryNotice,
  type TranscriptStreamStatus,
} from './useSessionTranscript';

vi.mock('@microsoft/fetch-event-source', () => ({
  fetchEventSource: vi.fn(() => new Promise<void>(() => undefined)),
}));

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  get length() {
    return this.values.size;
  }

  clear() {
    this.values.clear();
  }

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  key(index: number) {
    return [...this.values.keys()][index] ?? null;
  }

  removeItem(key: string) {
    this.values.delete(key);
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

interface HookState {
  entries: TranscriptEntry[];
  status: TranscriptStreamStatus;
  latencyMs?: number;
  recovery?: TranscriptRecoveryNotice;
}

const sessionAEntry: TranscriptEntry = {
  entry_id: 'a-1',
  sequence: 1,
  timestamp: '2026-08-13T00:00:00.000Z',
  role: 'assistant',
  content: '来自会话 A 的记录',
};

function snapshot(sessionId: string, entries: TranscriptEntry[]): TranscriptSnapshot {
  return {
    session_id: sessionId,
    entries,
    overflowed: false,
    oldest_sequence: entries[0]?.sequence,
    next_sequence: 1,
    stream_generation: 'test-generation',
    stream_next_sequence: 1,
  };
}

function HookProbe({
  sessionId,
  onState,
}: {
  sessionId: string;
  onState: (state: HookState) => void;
}) {
  const state = useSessionTranscript(sessionId);
  onState(state);
  return null;
}

async function flushAsyncWork() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('session transcript hook lifecycle', () => {
  beforeEach(() => {
    vi.stubGlobal('sessionStorage', new MemoryStorage());
    vi.stubGlobal('localStorage', new MemoryStorage());
    vi.stubGlobal('window', {
      setTimeout,
      dispatchEvent: vi.fn(),
    });
    saveAdminToken('test-token');
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('clears A before loading B so a failed B snapshot cannot retain A content', async () => {
    vi.spyOn(adminApi, 'transcript').mockImplementation(async (sessionId) => {
      if (sessionId === 'session-a') {
        return snapshot(sessionId, [sessionAEntry]);
      }
      throw new Error('session B unavailable');
    });

    let latest: HookState | undefined;
    const states: HookState[] = [];
    const onState = (state: HookState) => {
      latest = state;
      states.push(state);
    };
    let renderer: ReactTestRenderer;

    await act(async () => {
      renderer = create(createElement(HookProbe, { sessionId: 'session-a', onState }));
    });
    await flushAsyncWork();

    expect(latest?.entries).toEqual([sessionAEntry]);

    const sessionBStart = states.length;
    await act(async () => {
      renderer.update(createElement(HookProbe, { sessionId: 'session-b', onState }));
    });

    const firstSessionBState = states
      .slice(sessionBStart)
      .find((state) => state.entries.length === 0);
    expect(firstSessionBState).toMatchObject({ entries: [], status: 'loading' });
    expect(firstSessionBState?.latencyMs).toBeUndefined();
    expect(firstSessionBState?.recovery).toBeUndefined();

    await flushAsyncWork();

    expect(latest).toMatchObject({
      entries: [],
      status: 'recovery_needed',
      recovery: { kind: 'snapshot_failed' },
    });
    expect(latest?.latencyMs).toBeUndefined();

    await act(async () => {
      renderer.unmount();
    });
  });
});
