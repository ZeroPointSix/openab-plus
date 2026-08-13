export interface AgentLaunchDefinition {
  command: string;
  args: string[];
}

const AGENT_LAUNCH_DEFINITIONS: Record<string, AgentLaunchDefinition> = {
  claude: { command: 'claude-agent-acp', args: [] },
  codex: { command: 'codex-acp', args: [] },
  cursor: { command: 'cursor-agent', args: ['acp'] },
  gemini: { command: 'gemini', args: ['--acp'] },
  hermes: { command: 'hermes-acp', args: [] },
  kiro: { command: 'kiro-cli', args: ['acp', '--trust-all-tools'] },
  opencode: { command: 'opencode', args: ['acp'] },
};

/**
 * Returns a copy so form edits never mutate the registry shared by later wizard
 * instances. Unknown agent types require an explicit command from the operator.
 */
export function agentLaunchDefinition(
  agentType?: string,
): AgentLaunchDefinition | undefined {
  const definition = agentType
    ? AGENT_LAUNCH_DEFINITIONS[agentType.trim().toLowerCase()]
    : undefined;
  return definition
    ? { command: definition.command, args: [...definition.args] }
    : undefined;
}
