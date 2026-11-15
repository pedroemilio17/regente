import { describe, expect, it } from 'vitest';
import type { AgentEvent, AgentView, Task } from '../api/types';
import { agentStatus, executionEdges, inRun, runRoots, structureEdges, taskStatus } from './projection';

const NOW = Date.parse('2026-11-15T12:00:00Z');
const ago = (ms: number) => new Date(NOW - ms).toISOString();

const agent = (id: string, extra: Partial<AgentView> = {}, live: Partial<AgentView['live']> = {}) =>
  ({
    id,
    name: id,
    project_id: 'p1',
    parent_id: null,
    paused: false,
    is_orchestrator: false,
    live: { status: 'idle', doing_now: '', task_id: null, recent_changes: [], cost_usd: 0, tokens: 0, last_ts: null, pause_reason: null, ...live },
    ...extra,
  }) as AgentView;

const task = (id: string, owner: string, status: Task['status'], extra: Partial<Task> = {}) =>
  ({ id, owner_agent_id: owner, status, parent_task_id: null, depends_on: [], title: id, created_at: ago(1000), ...extra }) as Task;

let seq = 0;
const event = (agentId: string, kind: AgentEvent['kind'], extra: Partial<AgentEvent> = {}) =>
  ({ seq: ++seq, id: `e${seq}`, ts: ago(1000), agent_id: agentId, kind, summary: kind, ...extra }) as AgentEvent;

describe('taskStatus', () => {
  it('maps daemon statuses to UI statuses', () => {
    expect(taskStatus(task('t', 'a', 'done'))).toBe('completed');
    expect(taskStatus(task('t', 'a', 'waiting'))).toBe('review');
    expect(taskStatus(task('t', 'a', 'blocked'))).toBe('blocked');
    expect(taskStatus(task('t', 'a', 'queued'))).toBe('queued');
    expect(taskStatus(task('t', 'a', 'cancelled'))).toBe('cancelled');
  });
  it('is failed when an error event points at the open task', () => {
    expect(taskStatus(task('t', 'a', 'running'), [event('a', 'error', { task_id: 't' })])).toBe('failed');
  });
});

describe('agentStatus', () => {
  it('goes back to idle once the task is done', () => {
    const a = agent('a', {}, { status: 'idle', last_ts: ago(1000) });
    const result = agentStatus(a, [task('t', 'a', 'done')], [event('a', 'task_done', { task_id: 't', summary: 'Concluído' })], NOW);
    expect(result.status).toBe('idle');
    expect(result.currentTask).toBeNull();
    expect(result.lastActivity?.summary).toBe('Concluído');
  });
  it('is idle without tasks even if the last live status said working', () => {
    expect(agentStatus(agent('a', {}, { status: 'working', last_ts: ago(1000) }), [], [], NOW).status).toBe('idle');
  });
  it('works on a running task with recent activity', () => {
    const result = agentStatus(agent('a', {}, { status: 'working', task_id: 't', last_ts: ago(1000) }), [task('t', 'a', 'running')], [], NOW);
    expect(result.status).toBe('working');
    expect(result.currentTask?.id).toBe('t');
  });
  it('is unknown without a heartbeat', () => {
    expect(agentStatus(agent('a', {}, { status: 'working', last_ts: ago(10 * 60_000) }), [task('t', 'a', 'running')], [], NOW).status).toBe('unknown');
  });
  it('is paused', () => {
    expect(agentStatus(agent('a', { paused: true }, { status: 'paused' }), [task('t', 'a', 'running')], [], NOW).status).toBe('paused');
  });
  it('is error after an error event on the current task', () => {
    const events = [event('a', 'error', { task_id: 't', ts: ago(500) })];
    expect(agentStatus(agent('a', {}, { last_ts: ago(500) }), [task('t', 'a', 'running')], events, NOW).status).toBe('error');
  });
  it('is waiting on a task in review or blocked', () => {
    expect(agentStatus(agent('a'), [task('t', 'a', 'waiting')], [], NOW).status).toBe('waiting');
    expect(agentStatus(agent('a'), [task('t', 'a', 'blocked')], [], NOW).status).toBe('waiting');
  });
  it('is waiting without a task while the daemon waits for an approval', () => {
    expect(agentStatus(agent('a', {}, { status: 'waiting' }), [], [], NOW).status).toBe('waiting');
  });
  it('is offline when the stream is down', () => {
    expect(agentStatus(agent('a'), [], [], NOW, false).status).toBe('offline');
  });
});

describe('edges', () => {
  const team = [agent('m', { is_orchestrator: true }), agent('b', { parent_id: 'm' }), agent('c', { parent_id: 'm' })];

  it('structure edges come from parent_id', () => {
    expect(structureEdges(team).map((edge) => `${edge.source}>${edge.target}`)).toEqual(['m>b', 'm>c']);
  });

  it('agents without parent hang under the top orchestrator', () => {
    const flat = [agent('m', { is_orchestrator: true }), agent('b'), agent('c', { parent_id: 'b' })];
    expect(structureEdges(flat).map((edge) => `${edge.source}>${edge.target}`)).toEqual(['m>b', 'b>c']);
    expect(structureEdges([agent('x'), agent('y')])).toEqual([]);
  });

  it('a "vou delegar" message without task_assigned creates no edge', () => {
    const events = [event('m', 'message_sent', { target: 'b', summary: 'vou delegar o backend para b', data: { to: 'b', text: 'vou delegar' } })];
    expect(executionEdges(team, [task('root', 'm', 'running')], events)).toEqual([]);
  });

  it('task_assigned links the parent task owner to the recipient', () => {
    const tasks = [task('root', 'm', 'running'), task('t1', 'b', 'queued', { parent_task_id: 'root' })];
    const events = [event('m', 'task_assigned', { task_id: 'root' }), event('b', 'task_assigned', { task_id: 't1' })];
    expect(executionEdges(team, tasks, events)).toMatchObject([{ id: 'delegated:t1', kind: 'delegated', source: 'm', target: 'b', taskId: 't1' }]);
  });

  it('marks failed delegations and dependency edges', () => {
    const tasks = [
      task('root', 'm', 'running'),
      task('t1', 'b', 'running', { parent_task_id: 'root' }),
      task('t2', 'c', 'blocked', { parent_task_id: 'root', depends_on: ['t1'] }),
    ];
    const events = [event('b', 'task_assigned', { task_id: 't1' }), event('b', 'error', { task_id: 't1' })];
    const edges = executionEdges(team, tasks, events);
    expect(edges.find((edge) => edge.id === 'delegated:t1')?.kind).toBe('failed');
    expect(edges.find((edge) => edge.id === 'dep:t1:t2')).toMatchObject({ kind: 'blocked_by', source: 'b', target: 'c' });
  });
});

describe('runs', () => {
  it('picks active roots newest first, else the newest finished one', () => {
    const old = task('r1', 'o', 'running', { project_id: 'p1', created_at: ago(5000) });
    const recent = task('r2', 'o', 'queued', { project_id: 'p1', created_at: ago(1000) });
    const done = task('r3', 'o', 'done', { project_id: 'p1', created_at: ago(500) });
    const other = task('x', 'o', 'running', { project_id: 'p2' });
    expect(runRoots([old, recent, done, other], 'p1').map((t) => t.id)).toEqual(['r2', 'r1']);
    expect(runRoots([done, other], 'p1').map((t) => t.id)).toEqual(['r3']);
  });
  it('keeps only the edges of the chosen run', () => {
    const agents = [agent('o'), agent('w')];
    const tasks = [
      task('r1', 'o', 'running', { root_task_id: 'r1' }),
      task('c1', 'w', 'running', { parent_task_id: 'r1', root_task_id: 'r1' }),
      task('r2', 'o', 'running', { root_task_id: 'r2' }),
      task('c2', 'w', 'running', { parent_task_id: 'r2', root_task_id: 'r2' }),
    ];
    const events = [event('w', 'task_assigned', { task_id: 'c1' }), event('w', 'task_assigned', { task_id: 'c2' })];
    const run = inRun(tasks, events, 'r2');
    expect(executionEdges(agents, run.tasks, run.events).map((edge) => edge.id)).toEqual(['delegated:c2']);
  });
});
