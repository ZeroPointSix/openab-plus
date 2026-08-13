import { describe, expect, it } from 'vitest';
import type { TranscriptEntry } from '../types';
import {
  resetTranscriptView,
  type TranscriptViewState,
} from './useSessionTranscript';

const sessionAEntry: TranscriptEntry = {
  entry_id: 'a-1',
  sequence: 1,
  timestamp: '2026-08-13T00:00:00.000Z',
  role: 'assistant',
  content: '来自会话 A 的记录',
};

describe('session transcript view reset', () => {
  it('clears A before loading B so a failed B snapshot cannot retain A content', () => {
    const visibleForSessionA: TranscriptViewState = {
      entries: [sessionAEntry],
      latencyMs: 42,
      recovery: {
        kind: 'history_gap',
        message: '会话 A 需要恢复。',
        oldestSequence: 5,
      },
    };

    expect(visibleForSessionA.entries).toHaveLength(1);

    const visibleWhileLoadingSessionB = resetTranscriptView();

    expect(visibleWhileLoadingSessionB).toEqual({ entries: [] });
    expect(visibleWhileLoadingSessionB.latencyMs).toBeUndefined();
    expect(visibleWhileLoadingSessionB.recovery).toBeUndefined();

    // A B-snapshot failure leaves the just-reset view intact; it must not
    // resurrect the old transcript, latency observation, or recovery banner.
    const visibleAfterSessionBSnapshotFailure = visibleWhileLoadingSessionB;
    expect(visibleAfterSessionBSnapshotFailure.entries).toEqual([]);
    expect(visibleAfterSessionBSnapshotFailure.recovery).toBeUndefined();
  });
});
