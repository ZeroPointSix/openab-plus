import { describe, expect, it } from 'vitest';
import { agentLaunchDefinition } from './agentLaunch';

describe('agentLaunchDefinition', () => {
  it('maps a non-default Agent to its official ACP adapter command', () => {
    expect(agentLaunchDefinition('claude')).toEqual({
      command: 'claude-agent-acp',
      args: [],
    });
  });

  it('returns a copy of command arguments for each lookup', () => {
    const first = agentLaunchDefinition('opencode');
    first?.args.push('--unsafe');

    expect(agentLaunchDefinition('opencode')).toEqual({
      command: 'opencode',
      args: ['acp'],
    });
  });

  it('requires an explicit command for unknown Agent types', () => {
    expect(agentLaunchDefinition('custom-acp')).toBeUndefined();
  });
});
