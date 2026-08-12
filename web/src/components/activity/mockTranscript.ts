import type { ActivityEntry } from '../../types';
import { normalizeToolCall } from './normalizeToolCall';

const at = '2026-08-12T07:35:00.000Z';

const readTool = normalizeToolCall({
  call_id: 'tool-read-session',
  name: 'read',
  kind: 'read',
  status: 'completed',
  raw_input: { file_path: 'src/server/session.ts' },
  output: '读取到 214 行，会话快照接口尚未返回 transcript。',
  duration_ms: 186,
});

const editTool = normalizeToolCall({
  call_id: 'tool-edit-activity',
  name: 'edit',
  kind: 'edit',
  status: 'completed',
  raw_input: { file_path: 'web/src/components/activity/SessionActivityFeed.tsx' },
  duration_ms: 642,
  output: {
    path: 'web/src/components/activity/SessionActivityFeed.tsx',
    old_text: "return <Timeline items={items} />;",
    new_text: "return <SessionActivityFeed entries={entries} />;",
  },
});

const failedTool = normalizeToolCall({
  call_id: 'tool-search-schema',
  name: 'grep',
  kind: 'grep',
  status: 'error',
  raw_input: { pattern: 'transcript', path: 'src/api' },
  output: 'rg: src/api: No such file or directory',
  duration_ms: 23,
});

if (!readTool || !editTool || !failedTool) {
  throw new Error('Activity feed mock tools must be normalizable.');
}

export const mockTranscript: ActivityEntry[] = [
  { id: 'turn-1', type: 'turn', label: '回合 1', created_at: at },
  {
    id: 'user-1',
    type: 'user',
    text: '请检查这个 Slack 会话的运行状态，并定位最近一次失败原因。',
    created_at: at,
  },
  {
    id: 'assistant-1',
    type: 'assistant',
    text: '我会读取会话状态、检查近期日志，并将定位结果整理为可复核的步骤。',
    created_at: '2026-08-12T07:35:03.000Z',
  },
  {
    id: 'thinking-1',
    type: 'thinking',
    text: '先核对快照状态，再查询 transcript 契约；如果接口尚未就绪，保留组件级 mock 以避免阻塞 UI 工作。',
    created_at: '2026-08-12T07:35:04.000Z',
  },
  {
    id: 'plan-1',
    type: 'plan',
    title: '调查步骤',
    items: [
      { text: '读取会话快照', done: true },
      { text: '检索 transcript 契约', done: true },
      { text: '输出可复核结论' },
    ],
    created_at: '2026-08-12T07:35:05.000Z',
  },
  { id: 'tool-1', type: 'tool', tool: readTool, created_at: '2026-08-12T07:35:06.000Z' },
  {
    id: 'terminal-1',
    type: 'terminal',
    terminal: {
      command: 'cargo test -p openab-gateway transcript',
      output: 'running 2 tests\ntest transcript::snapshot ... ok\ntest transcript::stream ... ok\n',
      exit_code: 0,
    },
    created_at: '2026-08-12T07:35:07.000Z',
  },
  { id: 'tool-2', type: 'tool', tool: editTool, created_at: '2026-08-12T07:35:09.000Z' },
  { id: 'tool-3', type: 'tool', tool: failedTool, created_at: '2026-08-12T07:35:10.000Z' },
  {
    id: 'error-1',
    type: 'error',
    message: 'Transcript API 尚未接入此页面，当前显示固定 mock 数据。',
    created_at: '2026-08-12T07:35:11.000Z',
  },
  {
    id: 'assistant-2',
    type: 'assistant',
    text: '会话进程正常退出，相关测试通过；真实 transcript 将在 W3 接口可用后由同一渲染器接入。',
    created_at: '2026-08-12T07:35:12.000Z',
  },
  {
    id: 'tool-4',
    type: 'tool',
    tool: {
      key: 'tool-running-search',
      name: 'search',
      kind: 'search',
      status: 'running',
      description: '“SSE activity snapshot” in web/src',
      input: '{\n  "query": "SSE activity snapshot",\n  "path": "web/src"\n}',
    },
    created_at: '2026-08-12T07:35:13.000Z',
  },
];
