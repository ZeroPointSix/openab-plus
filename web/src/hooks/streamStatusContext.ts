import { createContext, useContext } from 'react';
import type { StreamStatus } from './useSessionStream';

export const streamStatusLabels: Record<StreamStatus, string> = {
  connecting: '正在连接',
  live: '实时连接',
  reconnecting: '正在重连',
  offline: '离线',
};

export const StreamStatusContext = createContext<StreamStatus>('offline');

export function useStreamStatus(): StreamStatus {
  return useContext(StreamStatusContext);
}
