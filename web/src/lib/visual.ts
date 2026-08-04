const ENTITY_PALETTE = [
  { color: '#1668dc', background: '#e8f1ff' },
  { color: '#1f9d68', background: '#e7f6ef' },
  { color: '#b96d08', background: '#fff3dd' },
  { color: '#7b5cd6', background: '#f0ebfc' },
  { color: '#c04a7c', background: '#fceef4' },
  { color: '#0e8a9d', background: '#e6f6f9' },
] as const;

export interface EntityVisual {
  color: string;
  background: string;
  initial: string;
}

/**
 * Deterministic color + initial for an entity name (agent, profile, ...),
 * so lists stay scannable without a per-entity icon set.
 */
export function visualForEntity(value?: string): EntityVisual {
  const label = (value || '').trim();
  let hash = 0;
  for (const char of label) {
    hash = (hash * 31 + (char.codePointAt(0) || 0)) >>> 0;
  }
  const entry = ENTITY_PALETTE[hash % ENTITY_PALETTE.length];
  const initial = (
    label.match(/[a-zA-Z0-9一-龥]/)?.[0] || '?'
  ).toUpperCase();
  return { color: entry.color, background: entry.background, initial };
}
