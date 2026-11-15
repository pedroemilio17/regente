// Simple top-down tree layout (no elkjs/dagre): each subtree gets the width of
// its leaves, parents are centered over their children. Pinned positions win.

export interface TreeItem {
  id: string;
  parentId: string | null;
}

export interface Point {
  x: number;
  y: number;
}

export const NODE_W = 256;
export const NODE_H = 150;
const GAP_X = 40;
const GAP_Y = 90;
/** More leaf children than this wrap into a grid instead of one very wide row. */
const MAX_ROW = 6;

/** Positions for a forest. Roots keep the input order (orchestrator first). */
export function treeLayout(items: TreeItem[]): Record<string, Point> {
  const ids = new Set(items.map((item) => item.id));
  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const item of items) {
    if (item.parentId && ids.has(item.parentId) && item.parentId !== item.id) {
      children.set(item.parentId, [...(children.get(item.parentId) ?? []), item.id]);
    } else {
      roots.push(item.id);
    }
  }

  const out: Record<string, Point> = {};
  const visiting = new Set<string>();
  // Returns the width used; `left` is where the subtree starts.
  const place = (id: string, depth: number, left: number): number => {
    visiting.add(id);
    const kids = (children.get(id) ?? []).filter((kid) => !visiting.has(kid));
    if (kids.length > MAX_ROW && kids.every((kid) => !children.has(kid))) {
      const cols = Math.max(4, Math.ceil(Math.sqrt(kids.length)));
      kids.forEach((kid, index) => {
        visiting.add(kid);
        out[kid] = { x: left + (index % cols) * (NODE_W + GAP_X), y: (depth + 1 + Math.floor(index / cols)) * (NODE_H + GAP_Y) };
      });
      const width = cols * NODE_W + (cols - 1) * GAP_X;
      out[id] = { x: left + (width - NODE_W) / 2, y: depth * (NODE_H + GAP_Y) };
      return width;
    }
    let width = 0;
    for (const kid of kids) {
      width += width > 0 ? GAP_X : 0;
      width += place(kid, depth + 1, left + width);
    }
    width = Math.max(width, NODE_W);
    out[id] = { x: left + (width - NODE_W) / 2, y: depth * (NODE_H + GAP_Y) };
    return width;
  };

  let left = 0;
  for (const root of roots) {
    left += place(root, 0, left) + GAP_X;
  }
  // Nodes caught in a parent cycle never reached from a root.
  for (const item of items) {
    if (!out[item.id]) {
      left += place(item.id, 0, left) + GAP_X;
    }
  }
  return out;
}

/** Layout with pinned nodes kept exactly where the user left them. */
export function placeNodes(items: TreeItem[], auto: Record<string, Point>, pinned: Record<string, Point>): Record<string, Point> {
  const out: Record<string, Point> = {};
  for (const item of items) {
    out[item.id] = pinned[item.id] ?? (auto[item.id] as Point);
  }
  return out;
}

/** Changes only when ids or parents change, so the layout is not redone on every event. */
export function topologyKey(items: TreeItem[]): string {
  return items.map((item) => `${item.id}<${item.parentId ?? ''}`).join('|');
}
