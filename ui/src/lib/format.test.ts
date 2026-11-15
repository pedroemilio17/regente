import { describe, expect, it } from 'vitest';
import { usageBar } from './format';

describe('usageBar', () => {
  it('shows the available share (100 - used) and the reset time', () => {
    const bar = usageBar('5h', { start: null, tokens: 0, cost_usd: 3, pct: 0.38, resets_at: new Date(Date.now() + 3600_000).toISOString() });
    expect(bar.value).toBe('62% disponível');
    expect(bar.width).toBe(62);
    expect(bar.reset).toMatch(/^reinicia/);
  });

  it('does not invent a number without a reported quota', () => {
    const bar = usageBar('week', { start: null, tokens: 0, cost_usd: 3, pct: null });
    expect(bar.value).toBe('sem dados de cota');
    expect(bar.width).toBeNull();
  });
});
