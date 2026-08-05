import { describe, expect, it } from 'vitest';
import {
  areConfigValuesEqual,
  buildConfigFields,
  editableSecretValue,
  findUnsafeIntegerPaths,
  getConfigValue,
  maskConfigSecrets,
  updateConfigValue,
} from './config';
import { ConfigMetadata, ConfigValues } from '../types';

const metadata: ConfigMetadata[] = [
  {
    path: 'telegram.bot_token',
    apply_policy: 'restart_required',
    secret: true,
  },
  {
    path: 'telegram.rich_messages',
    apply_policy: 'runtime',
    secret: false,
  },
];

describe('Gateway config editor helpers', () => {
  it('combines current values with metadata-only fields', () => {
    const fields = buildConfigFields(
      { telegram: { webhook_path: '/hooks/telegram' } },
      metadata,
    );

    expect(fields.map((field) => field.path)).toEqual([
      'telegram.bot_token',
      'telegram.rich_messages',
      'telegram.webhook_path',
    ]);
    expect(fields.find((field) => field.path === 'telegram.bot_token')).toMatchObject({
      kind: 'string',
      secret: true,
      value: undefined,
    });
    expect(fields.find((field) => field.path === 'telegram.rich_messages')).toMatchObject({
      kind: 'boolean',
      applyPolicy: 'runtime',
    });
  });

  it('updates a nested value without mutating the source document', () => {
    const source: ConfigValues = { telegram: { rich_messages: true } };
    const updated = updateConfigValue(
      source,
      'telegram.rich_messages',
      false,
    );

    expect(getConfigValue(source, 'telegram.rich_messages')).toBe(true);
    expect(getConfigValue(updated, 'telegram.rich_messages')).toBe(false);
  });

  it('masks secret values after a successful save', () => {
    const masked = maskConfigSecrets(
      { telegram: { bot_token: 'new-token', rich_messages: true } },
      metadata,
    );

    expect(getConfigValue(masked, 'telegram.bot_token')).toBe('********');
    expect(getConfigValue(masked, 'telegram.rich_messages')).toBe(true);
  });

  it('keeps masked secrets out of the editable input value', () => {
    expect(editableSecretValue('********')).toBe('');
    expect(editableSecretValue('new-token')).toBe('new-token');
  });

  it('masks heuristic secrets nested inside arrays', () => {
    const masked = maskConfigSecrets(
      {
        providers: [
          { name: 'first', password: 'plain-password' },
          { name: 'second', options: { access_token: 'plain-token' } },
        ],
      },
      [],
    );

    expect(masked.providers).toEqual([
      { name: 'first', password: '********' },
      { name: 'second', options: { access_token: '********' } },
    ]);
  });

  it('reports unsafe integers before a document can be saved', () => {
    expect(
      findUnsafeIntegerPaths({
        gateway: { request_id: Number.MAX_SAFE_INTEGER + 1 },
        providers: [{ timeout: 30 }, { account_id: 9_007_199_254_740_992 }],
      }),
    ).toEqual(['gateway.request_id', 'providers[1].account_id']);
  });

  it('compares documents independently of object key order', () => {
    expect(
      areConfigValuesEqual(
        { gateway: { url: 'wss://example.com', platform: 'telegram' } },
        { gateway: { platform: 'telegram', url: 'wss://example.com' } },
      ),
    ).toBe(true);
  });
});
