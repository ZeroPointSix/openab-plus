import { describe, expect, it } from 'vitest';
import { normalizeProfilePayload } from './profile';

describe('profile payload normalization', () => {
  it('converts dynamic values and reference rows to API records', () => {
    const profile = normalizeProfilePayload({
      id: ' codex-main ',
      name: ' Codex Main ',
      agent_type: 'codex',
      command: ' codex-acp ',
      enabled: true,
      workdir_strategy: 'profile_default',
      recovery_strategy: 'resume_session',
      timeout_secs: 120,
      args: ['--quiet', ''],
      env_ref_entries: [
        { key: 'OPENAI_API_KEY', value: 'credstore://openai.key' },
      ],
      config_options: { model: 'gpt-5', reasoning: 3 },
    });

    expect(profile.id).toBe('codex-main');
    expect(profile.command).toBe('codex-acp');
    expect(profile.args).toEqual(['--quiet']);
    expect(profile.env_refs).toEqual({
      OPENAI_API_KEY: 'credstore://openai.key',
    });
    expect(profile.config_options).toEqual({
      model: 'gpt-5',
      reasoning: '3',
    });
  });
});
