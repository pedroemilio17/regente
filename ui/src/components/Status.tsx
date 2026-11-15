import { memo } from 'react';
import { useAnimationGate } from '../hooks/useAnimationGate';
import { statusStyle } from '../lib/labels';

/** Small status dot; it pulses only for working and thinking. */
export const StatusDot = memo(function StatusDot({ status, title }: { status: string; title?: boolean }) {
  const gate = useAnimationGate<HTMLSpanElement>();
  const style = statusStyle(status);
  return (
    <span
      ref={gate}
      className={`dot dot-${style.key}${style.live ? ' live' : ''}`}
      title={title ? style.label : undefined}
    />
  );
});

export const StatusPill = memo(function StatusPill({ status }: { status: string }) {
  const style = statusStyle(status);
  return (
    <span className={`pill st-${style.key}`} data-testid="status-pill">
      <StatusDot status={status} />
      {style.label}
    </span>
  );
});
