import { describe, expect, it } from 'vitest';
import { NODE_H, placeNodes, topologyKey, treeLayout } from './layout';

describe('treeLayout', () => {
  it('puts the orchestrator on top, centered over its children', () => {
    const pos = treeLayout([
      { id: 'm', parentId: null },
      { id: 'a', parentId: 'm' },
      { id: 'b', parentId: 'm' },
    ]);
    expect(pos.m?.y).toBe(0);
    expect(pos.a?.y).toBeGreaterThan(NODE_H);
    expect(pos.a?.y).toBe(pos.b?.y);
    expect(pos.a!.x).toBeLessThan(pos.b!.x);
    expect(pos.m!.x).toBe((pos.a!.x + pos.b!.x) / 2);
  });

  it('wraps many leaf children into rows', () => {
    const items = [{ id: 'm', parentId: null }, ...Array.from({ length: 30 }, (_, i) => ({ id: `a${i}`, parentId: 'm' }))];
    const pos = treeLayout(items);
    const rows = new Set(items.slice(1).map((item) => pos[item.id]!.y));
    expect(rows.size).toBe(5);
    expect(new Set(Object.values(pos).map((p) => `${p.x},${p.y}`)).size).toBe(31);
  });

  it('survives a parent cycle', () => {
    const pos = treeLayout([
      { id: 'a', parentId: 'b' },
      { id: 'b', parentId: 'a' },
    ]);
    expect(Object.keys(pos).sort()).toEqual(['a', 'b']);
  });
});

describe('placeNodes', () => {
  it('a new agent does not move a pinned node', () => {
    const before = [
      { id: 'm', parentId: null },
      { id: 'a', parentId: 'm' },
    ];
    const pinned = { a: { x: 900, y: 40 } };
    const after = [...before, { id: 'b', parentId: 'm' }];
    expect(placeNodes(before, treeLayout(before), pinned).a).toEqual({ x: 900, y: 40 });
    expect(placeNodes(after, treeLayout(after), pinned).a).toEqual({ x: 900, y: 40 });
    expect(placeNodes(after, treeLayout(after), pinned).b).toBeDefined();
  });
});

describe('topologyKey', () => {
  it('changes only with ids and parents', () => {
    const items = [{ id: 'm', parentId: null }];
    expect(topologyKey(items)).toBe(topologyKey([{ id: 'm', parentId: null }]));
    expect(topologyKey(items)).not.toBe(topologyKey([...items, { id: 'a', parentId: 'm' }]));
  });
});
