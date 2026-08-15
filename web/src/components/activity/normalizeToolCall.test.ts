import { describe, expect, it } from 'vitest';
import {
  buildParamSummary,
  normalizeAcpToolCall,
  normalizeToolCall,
  normalizeToolStatus,
} from './normalizeToolCall';

describe('activity tool-call normalization', () => {
  it.each([
    ['Success', 'completed'],
    ['in_progress', 'running'],
    ['failed', 'error'],
    ['Canceled', 'canceled'],
    ['unknown', 'pending'],
  ] as const)('maps %s to %s', (source, expected) => {
    expect(normalizeToolStatus(source)).toBe(expected);
  });

  it('summarizes supported filesystem, command, and search inputs', () => {
    expect(buildParamSummary('read', { file_path: 'src/main.ts' })).toBe('src/main.ts');
    expect(buildParamSummary('execute', { command: 'pnpm lint' })).toBe('pnpm lint');
    expect(buildParamSummary('grep', { pattern: 'transcript', path: 'web/src' })).toBe(
      '“transcript” in web/src',
    );
    expect(buildParamSummary('glob', { pattern: '**/*.tsx', path: 'web/src' })).toBe(
      '**/*.tsx in web/src',
    );
  });

  it('normalizes an edit result with a file diff', () => {
    const tool = normalizeToolCall({
      call_id: 'edit-1',
      name: 'edit',
      kind: 'edit',
      status: 'completed',
      raw_input: { path: 'src/page.tsx' },
      output: {
        path: 'src/page.tsx',
        old_text: 'before',
        new_text: 'after',
      },
      duration_ms: 400,
    });

    expect(tool).toMatchObject({
      key: 'edit-1',
      status: 'completed',
      description: 'src/page.tsx',
      duration_ms: 400,
      diff: { path: 'src/page.tsx', old_text: 'before', new_text: 'after' },
    });
  });

  it('maps execute tools to bash when the payload has no name', () => {
    const tool = normalizeToolCall({
      call_id: 'bash-1',
      kind: 'execute',
      status: 'completed',
      rawInput: { command: 'ls' },
    });
    expect(tool?.name).toBe('bash');
    expect(tool?.description).toBe('ls');
  });

  it.each(['execute', 'exec'] as const)(
    'maps an explicit %s name to bash', 
    (name) => {
      const tool = normalizeToolCall({
        call_id: `${name}-named`,
        name,
        status: 'completed',
        rawInput: { command: 'ls' },
      });
      expect(tool?.name).toBe('bash');
      expect(tool?.description).toBe('ls');
    },
  );

  it('normalizes ACP updates and text output', () => {
    const tool = normalizeAcpToolCall({
      content: {
        update: {
          tool_call_id: 'acp-1',
          title: '搜索会话事件',
          kind: 'search',
          status: 'in_progress',
          raw_input: { query: 'SSE', path: 'web/src' },
          content: [{ type: 'content', content: { text: '已找到 3 处匹配。' } }],
        },
      },
    });

    expect(tool).toMatchObject({
      key: 'acp-1',
      name: 'search',
      status: 'running',
      description: '“SSE” in web/src',
      output: '已找到 3 处匹配。',
    });
  });
});


describe('ACP file diff normalization', () => {
  it('normalizes an accumulated ACP output.diff using before/after fields', () => {
    const tool = normalizeToolCall({
      call_id: 'acp-diff-accumulated',
      title: '编辑文件',
      kind: 'edit',
      status: 'completed',
      rawInput: { path: 'src/lib.rs' },
      output: {
        status: 'completed',
        diff: {
          path: 'src/lib.rs',
          before: 'old',
          after: 'new',
        },
      },
    });

    expect(tool?.diffs).toEqual([
      { path: 'src/lib.rs', old_text: 'old', new_text: 'new' },
    ]);
  });

  it('preserves every diff item reported through update.content', () => {
    const tool = normalizeAcpToolCall({
      content: {
        update: {
          tool_call_id: 'acp-diff-1',
          title: '编辑会话活动流',
          kind: 'edit',
          status: 'completed',
          raw_input: { file_path: 'web/src/pages/SessionDetailPage.tsx' },
          content: [
            {
              type: 'diff',
              path: 'web/src/pages/SessionDetailPage.tsx',
              old_text: 'return <Timeline />;',
              new_text: 'return <SessionActivityFeed />;',
            },
            {
              type: 'diff',
              path: 'web/src/styles.css',
              old_text: '.timeline {}',
              new_text: '.activity-feed {}',
            },
          ],
        },
      },
    });

    expect(tool?.diff).toMatchObject({
      path: 'web/src/pages/SessionDetailPage.tsx',
      new_text: 'return <SessionActivityFeed />;',
    });
    expect(tool?.diffs).toEqual([
      {
        path: 'web/src/pages/SessionDetailPage.tsx',
        old_text: 'return <Timeline />;',
        new_text: 'return <SessionActivityFeed />;',
      },
      {
        path: 'web/src/styles.css',
        old_text: '.timeline {}',
        new_text: '.activity-feed {}',
      },
    ]);
  });
});
