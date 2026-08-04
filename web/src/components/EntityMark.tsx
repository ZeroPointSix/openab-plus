import { visualForEntity } from '../lib/visual';

interface EntityMarkProps {
  name?: string;
  size?: number;
}

export function EntityMark({ name, size = 26 }: EntityMarkProps) {
  const visual = visualForEntity(name);
  return (
    <span
      className="entity-mark"
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        color: visual.color,
        background: visual.background,
        fontSize: Math.round(size * 0.46),
      }}
    >
      {visual.initial}
    </span>
  );
}
