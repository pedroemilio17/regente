import { memo } from 'react';
import type { UsageWindows } from '../api/types';
import { usageBars } from '../lib/format';

/** Share still available in the 5h and weekly windows, as reported by the CLI. */
export const UsageBars = memo(function UsageBars({ windows }: { windows: UsageWindows | undefined }) {
  return (
    <div className="usage-rows">
      {usageBars(windows).map((bar) => (
        <div key={bar.key} style={{ display: 'contents' }}>
          <span>{bar.label}</span>
          <span className="bar">{bar.width !== null && <span style={{ width: `${bar.width}%` }} />}</span>
          <span className="value" title={bar.reset || undefined}>
            {bar.value}
            {bar.reset && <span className="muted"> · {bar.reset}</span>}
          </span>
        </div>
      ))}
    </div>
  );
});
