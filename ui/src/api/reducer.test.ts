import { describe, expect, it } from 'vitest';
import { applyEventToLive, applyEvents, emptyState, idleLive, mergeAlerts, RECENT_CHANGES } from './reducer';
import type { AgentEvent, AgentView } from './types';

const event = (seq: number, kind: AgentEvent['kind'], extra: Partial<AgentEvent> = {}): AgentEvent => ({
  seq, kind, id: `event-${seq}`, ts: `2026-01-01T00:00:${String(seq).padStart(2, '0')}Z`, agent_id: 'a', summary: kind, ...extra,
});

const agent = { id: 'a', live: idleLive(), paused: false, pause_reason: null } as AgentView;

describe('live event reduction', () => {
  it('keeps a budget pause while recording subsequent work and usage', () => {
    const paused = applyEventToLive(idleLive(), event(1, 'status', { data: { status: 'paused', reason: 'orçamento esgotado' } }));
    const next = applyEventToLive(paused, event(2, 'file_edit', { target: 'src/app.ts', usage: { tokens_in: 10, tokens_out: 5, tokens_cache: 0, cost_usd: 0.2, cost_source: 'estimated' } }));
    expect(next).toMatchObject({ status: 'paused', pause_reason: 'orçamento esgotado', tokens: 15, cost_usd: 0.2 });
    expect(next.recent_changes[0]?.path).toBe('src/app.ts');
  });

  it('bounds recent changes', () => {
    let live = idleLive();
    for (let i = 1; i <= RECENT_CHANGES + 2; i++) live = applyEventToLive(live, event(i, 'file_edit', { target: `${i}.ts` }));
    expect(live.recent_changes).toHaveLength(RECENT_CHANGES);
    expect(live.recent_changes[0]?.path).toBe('22.ts');
  });

  it('ignores replayed events and preserves unaffected agents', () => {
    const other = { ...agent, id: 'b' };
    const state = { ...emptyState, lastSeq: 2, agents: { a: agent, b: other } };
    expect(applyEvents(state, [event(2, 'thinking')])).toBe(state);
    const next = applyEvents(state, [event(2, 'thinking'), event(3, 'thinking')]);
    expect(next.lastSeq).toBe(3);
    expect(next.agents.a?.live.status).toBe('thinking');
    expect(next.agents.b).toBe(other);
  });

  it('deduplicates and orders alerts', () => {
    const old = { id: 'x', ts: '2026-01-01', agentId: 'a', summary: 'old', critical: false };
    const newest = { ...old, id: 'y', ts: '2026-01-02' };
    expect(mergeAlerts([old], [newest, old]).map((item) => item.id)).toEqual(['y', 'x']);
  });
});
