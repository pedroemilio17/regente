// Pure projections of daemon state into what the canvas shows: agent status,
// task status in UI terms and the edges of the graph. No React, no store.

import type { AgentEvent, AgentView, Task } from '../api/types';

export type AgentStatus = 'idle' | 'working' | 'waiting' | 'paused' | 'error' | 'offline' | 'unknown';
export type TaskStatus = 'queued' | 'running' | 'review' | 'blocked' | 'completed' | 'cancelled' | 'failed';

/** Working with no sign of life for this long is shown as unknown. */
export const STALE_MS = 5 * 60_000;

const TASK_MAP: Record<Task['status'], TaskStatus> = {
  queued: 'queued',
  running: 'running',
  waiting: 'review',
  blocked: 'blocked',
  done: 'completed',
  cancelled: 'cancelled',
};

export const TASK_STATUS_LABEL: Record<TaskStatus, string> = {
  queued: 'Na fila',
  running: 'Em andamento',
  review: 'Em revisão',
  blocked: 'Bloqueada',
  completed: 'Concluída',
  cancelled: 'Cancelada',
  failed: 'Falhou',
};

export const AGENT_STATUS_LABEL: Record<AgentStatus, string> = {
  idle: 'Disponível',
  working: 'Trabalhando',
  waiting: 'Aguardando',
  paused: 'Pausado',
  error: 'Erro',
  offline: 'Sem conexão',
  unknown: 'Sem atualização',
};

/** Shape per status, so status never depends on color alone. */
export const AGENT_STATUS_SHAPE: Record<AgentStatus, string> = {
  idle: '○',
  working: '●',
  waiting: '◐',
  paused: '‖',
  error: '✕',
  offline: '◌',
  unknown: '?',
};

const OPEN: Task['status'][] = ['running', 'waiting', 'blocked', 'queued'];

function failedTask(task: Task, events: AgentEvent[]): boolean {
  return task.status !== 'done' && events.some((event) => event.kind === 'error' && event.task_id === task.id);
}

export function taskStatus(task: Task, events: AgentEvent[] = []): TaskStatus {
  return failedTask(task, events) ? 'failed' : TASK_MAP[task.status];
}

export interface AgentProjection {
  status: AgentStatus;
  currentTask: Task | null;
  lastActivity: { summary: string; ts: string } | null;
}

/**
 * Status of one agent. Without an open task the agent is idle, whatever the
 * last event said: a finished task never leaves the agent "done" forever.
 */
export function agentStatus(
  agent: AgentView,
  tasks: Task[],
  events: AgentEvent[],
  now: number,
  connected = true,
): AgentProjection {
  const own = events.filter((event) => event.agent_id === agent.id);
  const last = own.reduce<AgentEvent | undefined>((best, event) => (!best || event.seq > best.seq ? event : best), undefined);
  const lastActivity =
    last && (!agent.live.last_ts || last.ts >= agent.live.last_ts)
      ? { summary: last.summary, ts: last.ts }
      : agent.live.last_ts
        ? { summary: agent.live.doing_now, ts: agent.live.last_ts }
        : null;

  const open = tasks.filter((task) => task.owner_agent_id === agent.id && OPEN.includes(task.status));
  const currentTask =
    open.find((task) => task.id === agent.live.task_id) ??
    [...open].sort((a, b) => OPEN.indexOf(a.status) - OPEN.indexOf(b.status))[0] ??
    null;

  const result = (status: AgentStatus): AgentProjection => ({ status, currentTask, lastActivity });
  if (!connected) {
    return result('offline');
  }
  if (agent.paused || agent.live.status === 'paused') {
    return result('paused');
  }
  if (!currentTask) {
    // No open task but the daemon still waits on the user (e.g. a merge approval).
    return result(agent.live.status === 'waiting' ? 'waiting' : 'idle');
  }
  if (last?.kind === 'error' || failedTask(currentTask, events)) {
    return result('error');
  }
  if (currentTask.status !== 'running' || agent.live.status === 'waiting') {
    return result('waiting');
  }
  const seen = lastActivity ? new Date(lastActivity.ts).getTime() : Number.NaN;
  if (!Number.isFinite(seen) || now - seen > STALE_MS) {
    return result('unknown');
  }
  return result('working');
}

export type EdgeKind = 'allowed' | 'delegated' | 'depends_on' | 'blocked_by' | 'failed';

export interface FlowEdge {
  id: string;
  kind: EdgeKind;
  source: string;
  target: string;
  taskId?: string;
  ts?: string;
}

/**
 * Configured parent. Agents created with the project have no `parent_id`, but
 * the daemon lets the top orchestrator delegate to them, so they hang under it.
 */
export function parentMap(agents: AgentView[]): Map<string, string | null> {
  const ids = new Set(agents.map((agent) => agent.id));
  const lead = agents.find((agent) => agent.is_orchestrator && !agent.parent_id)?.id ?? null;
  return new Map(
    agents.map((agent) => {
      if (agent.parent_id && agent.parent_id !== agent.id && ids.has(agent.parent_id)) return [agent.id, agent.parent_id];
      return [agent.id, agent.is_orchestrator || agent.id === lead ? null : lead];
    }),
  );
}

/** Configured topology: one edge from each parent to its child. */
export function structureEdges(agents: AgentView[]): FlowEdge[] {
  const parents = parentMap(agents);
  return agents.flatMap((agent) => {
    const parent = parents.get(agent.id);
    return parent ? [{ id: `allowed:${agent.id}`, kind: 'allowed' as const, source: parent, target: agent.id }] : [];
  });
}

/**
 * What actually happened. Delegations come only from `task_assigned` events
 * with a task id (emitted by the daemon's MCP), never from message text.
 */
export function executionEdges(agents: AgentView[], tasks: Task[], events: AgentEvent[]): FlowEdge[] {
  const ids = new Set(agents.map((agent) => agent.id));
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const edges: FlowEdge[] = [];
  const seen = new Set<string>();
  const add = (edge: FlowEdge) => {
    if (!seen.has(edge.id) && edge.source !== edge.target && ids.has(edge.source) && ids.has(edge.target)) {
      seen.add(edge.id);
      edges.push(edge);
    }
  };

  for (const event of events) {
    if (event.kind !== 'task_assigned' || !event.task_id) {
      continue;
    }
    const task = byId.get(event.task_id);
    const parentId = task?.parent_task_id ?? (typeof event.data?.parent_task_id === 'string' ? event.data.parent_task_id : null);
    const parent = parentId ? byId.get(parentId) : undefined;
    const source = parent?.owner_agent_id ?? (typeof event.data?.delegated_by === 'string' ? event.data.delegated_by : null);
    if (!source) {
      continue; // A task from the user is not a delegation.
    }
    const failed = task ? taskStatus(task, events) === 'failed' : false;
    add({ id: `delegated:${event.task_id}`, kind: failed ? 'failed' : 'delegated', source, target: event.agent_id, taskId: event.task_id, ts: event.ts });
  }

  for (const task of tasks) {
    for (const depId of task.depends_on) {
      const dep = byId.get(depId);
      if (!dep) {
        continue;
      }
      const blocked = task.status === 'blocked' && dep.status !== 'done';
      add({ id: `dep:${depId}:${task.id}`, kind: blocked ? 'blocked_by' : 'depends_on', source: dep.owner_agent_id, target: task.owner_agent_id, taskId: task.id, ts: task.created_at });
    }
  }
  return edges;
}

/** Runs of a project (root tasks): active ones newest first, else only the newest finished one. */
export function runRoots(tasks: Task[], projectId: string): Task[] {
  const roots = tasks
    .filter((task) => task.project_id === projectId && !task.parent_task_id)
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
  const active = roots.filter((task) => task.status !== 'done' && task.status !== 'cancelled');
  return active.length > 0 ? active : roots.slice(0, 1);
}

/** Tasks and task events of one run: the root task and everything under it. */
export function inRun(tasks: Task[], events: AgentEvent[], rootId: string): { tasks: Task[]; events: AgentEvent[] } {
  const run = tasks.filter((task) => task.id === rootId || task.root_task_id === rootId);
  const ids = new Set(run.map((task) => task.id));
  return { tasks: run, events: events.filter((event) => event.task_id != null && ids.has(event.task_id)) };
}
