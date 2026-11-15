// Accessible alternative to the graph: the same agents as a keyboard-navigable
// tree (flat treeitems with aria-level). Selection follows focus; Enter opens.

import { useRef, type KeyboardEvent } from 'react';
import type { AgentEvent, AgentView } from '../api/types';
import { useNow } from '../hooks/useNow';
import { formatAgo } from '../lib/format';
import { agentStatus, AGENT_STATUS_LABEL, AGENT_STATUS_SHAPE, parentMap } from '../lib/projection';
import { navigate } from '../lib/route';
import { useStore } from '../store';

interface Row {
  agent: AgentView;
  level: number;
  parentId: string | null;
}

/** Depth-first order, parents before children; cycles and orphans become roots. */
function flatten(agents: AgentView[]): Row[] {
  const parents = parentMap(agents);
  const kids = new Map<string, AgentView[]>();
  for (const agent of agents) {
    const parent = parents.get(agent.id);
    if (parent) kids.set(parent, [...(kids.get(parent) ?? []), agent]);
  }
  const rows: Row[] = [];
  const seen = new Set<string>();
  const walk = (agent: AgentView, level: number, parentId: string | null) => {
    if (seen.has(agent.id)) return;
    seen.add(agent.id);
    rows.push({ agent, level, parentId });
    (kids.get(agent.id) ?? []).forEach((kid) => walk(kid, level + 1, agent.id));
  };
  agents.filter((agent) => !parents.get(agent.id)).forEach((agent) => walk(agent, 1, null));
  agents.forEach((agent) => walk(agent, 1, null));
  return rows;
}

export function AgentList({ agents, events }: { agents: AgentView[]; events: AgentEvent[] }) {
  const now = useNow();
  const tasks = useStore((state) => state.tasks);
  const connected = useStore((state) => state.connection === 'online' || state.connection === 'degraded');
  const selectedAgentId = useStore((state) => state.selectedAgentId);
  const setSelectedAgentId = useStore((state) => state.setSelectedAgentId);
  const tree = useRef<HTMLUListElement>(null);

  const rows = flatten(agents);
  const activeId = rows.some((row) => row.agent.id === selectedAgentId) ? selectedAgentId : rows[0]?.agent.id;

  const move = (id: string | undefined) => {
    if (!id) return;
    setSelectedAgentId(id);
    const items = tree.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [];
    [...items].find((item) => item.dataset.id === id)?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    const index = rows.findIndex((row) => row.agent.id === activeId);
    const row = rows[index];
    if (!row) return;
    const target: Record<string, string | undefined> = {
      ArrowDown: rows[index + 1]?.agent.id,
      ArrowUp: rows[index - 1]?.agent.id,
      Home: rows[0]?.agent.id,
      End: rows[rows.length - 1]?.agent.id,
      ArrowLeft: row.parentId ?? undefined,
      ArrowRight: rows[index + 1]?.parentId === row.agent.id ? rows[index + 1]?.agent.id : undefined,
    };
    if (event.key in target) {
      event.preventDefault();
      move(target[event.key]);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      navigate({ screen: 'agent', agentId: row.agent.id });
    } else if (event.key === 'Escape') {
      setSelectedAgentId(null);
    }
  };

  return (
    <ul className="cv-list" role="tree" aria-label="Agentes do projeto" ref={tree} onKeyDown={onKeyDown} data-testid="agent-list">
      {rows.map(({ agent, level }) => {
        const { status, currentTask, lastActivity } = agentStatus(agent, tasks, events, now, connected);
        return (
          <li
            key={agent.id}
            role="treeitem"
            aria-level={level}
            aria-selected={agent.id === selectedAgentId}
            tabIndex={agent.id === activeId ? 0 : -1}
            data-id={agent.id}
            className={`cv-li cv-st-${status}`}
            style={{ paddingLeft: `${12 + (level - 1) * 24}px` }}
            onClick={() => move(agent.id)}
            onDoubleClick={() => navigate({ screen: 'agent', agentId: agent.id })}
          >
            <div className="cv-li-head">
              <b>{agent.name}</b>
              <span className="muted">{agent.role}</span>
              <span className="cv-status">
                <span className="cv-shape" aria-hidden="true">
                  {AGENT_STATUS_SHAPE[status]}
                </span>
                {AGENT_STATUS_LABEL[status]}
              </span>
            </div>
            <div className="cv-task">{currentTask ? `Tarefa: ${currentTask.title}` : 'Sem tarefa atual'}</div>
            {lastActivity && (
              <div className="cv-last">
                <span className="cv-last-text">{lastActivity.summary || 'Atividade'}</span>
                <span className="cv-ago">{formatAgo(lastActivity.ts, now)}</span>
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
