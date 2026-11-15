import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { AgentEvent, AgentView, Project, Task, TimelinePage } from '../api/types';
import { useStore } from '../store';
import { Overview } from './Overview';

const agent = (id: string, name: string, orchestrator: boolean) =>
  ({ id, name, role: orchestrator ? 'Orquestrador' : 'Backend', parent_id: orchestrator ? null : 'a2', project_id: 'p1', cli: 'claude', is_orchestrator: orchestrator, pet: {}, live: { status: 'idle', cost_usd: 0, recent_changes: [], last_ts: null } }) as unknown as AgentView;

const listeners = vi.hoisted(() => new Set<(batch: AgentEvent[]) => void>());
vi.mock('../api/stream', () => ({
  subscribeEvents: (listener: (batch: AgentEvent[]) => void) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
}));
let seq = 0;
const event = (agentId: string, summary: string) =>
  ({ seq: ++seq, id: `e${seq}`, ts: new Date().toISOString(), agent_id: agentId, kind: 'status', summary }) as AgentEvent;
const emit = (batch: AgentEvent[]) => act(() => listeners.forEach((listener) => listener(batch)));

// React Flow measures its container; jsdom has no ResizeObserver.
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= NoopResizeObserver as unknown as typeof ResizeObserver;

beforeEach(() => {
  localStorage.clear();
  vi.spyOn(api, 'timeline').mockResolvedValue({ items: [], next_before: null });
  useStore.setState({
    projects: [{ id: 'p1', name: 'Projeto' } as Project],
    agents: { a1: agent('a1', 'Ana', false), a2: agent('a2', 'Maestro', true) },
    agentOrder: ['a1', 'a2'],
    tasks: [],
    connection: 'online',
    selectedAgentId: null,
    focusTaskId: null,
  });
});
afterEach(cleanup);

it('defaults to the orchestrator and sends the task to the chosen agent', async () => {
  const create = vi.spyOn(api, 'createTask').mockResolvedValue({ title: 'x' } as Task);
  vi.spyOn(api, 'listTasks').mockResolvedValue([]);
  render(<Overview projectId="p1" onCreate={() => {}} />);
  const select = screen.getByLabelText('Para') as HTMLSelectElement;
  expect(select.value).toBe('a2');
  expect(select.textContent).toContain('Ana · Claude Code · Disponível');
  fireEvent.change(select, { target: { value: 'a1' } });
  fireEvent.change(screen.getByPlaceholderText('Descreva o que o time deve fazer'), { target: { value: 'Fazer' } });
  fireEvent.click(screen.getByRole('button', { name: 'Enviar tarefa' }));
  await waitFor(() => expect(create).toHaveBeenCalledWith({ project_id: 'p1', title: 'Fazer', budget_usd: null, agent_id: 'a1' }));
});

it('shows the team on the canvas instead of cards', () => {
  const onCreate = vi.fn();
  render(<Overview projectId="p1" onCreate={onCreate} />);
  const nodes = screen.getAllByTestId('canvas-node');
  expect(nodes.map((node) => node.getAttribute('data-agent'))).toEqual(['Maestro', 'Ana']);
  expect(screen.queryByTestId('agent-card')).toBeNull();
  expect(nodes[1]?.textContent).toContain('Disponível');
  fireEvent.click(screen.getByRole('button', { name: 'Novo agente' }));
  expect(onCreate).toHaveBeenCalledWith('agent');
});

it('selects on click, clears on Esc and switches to Execution', () => {
  render(<Overview projectId="p1" onCreate={() => {}} />);
  fireEvent.click(screen.getAllByTestId('canvas-node')[1] as HTMLElement);
  expect(useStore.getState().selectedAgentId).toBe('a1');
  fireEvent.keyDown(window, { key: 'Escape' });
  expect(useStore.getState().selectedAgentId).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Execução' }));
  expect(screen.getByRole('button', { name: 'Execução' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.queryByRole('button', { name: 'Novo agente' })).toBeNull();
  expect(screen.getAllByTestId('canvas-node')).toHaveLength(2);
});

it('shows the same agents as a keyboard tree and remembers the choice', () => {
  render(<Overview projectId="p1" onCreate={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Lista' }));
  expect(JSON.parse(localStorage.getItem('regente.canvas.p1') ?? '{}').view).toBe('list');
  const items = screen.getAllByRole('treeitem');
  expect(items.map((item) => item.getAttribute('aria-level'))).toEqual(['1', '2']);
  expect(items[0]?.textContent).toContain('Maestro');
  expect(items[1]?.textContent).toContain('Disponível');
  expect(items.map((item) => item.tabIndex)).toEqual([0, -1]);
  items[0]?.focus();
  fireEvent.keyDown(items[0] as HTMLElement, { key: 'ArrowDown' });
  expect(useStore.getState().selectedAgentId).toBe('a1');
  expect(document.activeElement).toBe(items[1]);
  fireEvent.keyDown(items[1] as HTMLElement, { key: 'ArrowLeft' });
  expect(useStore.getState().selectedAgentId).toBe('a2');
  fireEvent.keyDown(items[0] as HTMLElement, { key: 'Enter' });
  expect(window.location.hash).toBe('#/agente/a2');
  cleanup();
  render(<Overview projectId="p1" onCreate={() => {}} />);
  expect(screen.getByTestId('agent-list')).toBeTruthy();
});

it('never shows events of project A on the canvas of project B', async () => {
  localStorage.setItem('regente.canvas.p2', JSON.stringify({ view: 'list' }));
  useStore.setState({
    projects: [{ id: 'p1', name: 'A' } as Project, { id: 'p2', name: 'B' } as Project],
    agents: { a1: agent('a1', 'Ana', true), b1: { ...agent('b1', 'Bia', true), project_id: 'p2' } },
    agentOrder: ['a1', 'b1'],
  });
  let lateA: (page: TimelinePage) => void = () => {};
  vi.spyOn(api, 'timeline').mockImplementation((id) =>
    id === 'a1' ? new Promise((resolve) => (lateA = resolve)) : Promise.resolve({ items: [], next_before: null }),
  );
  const view = render(<Overview projectId="p1" onCreate={() => {}} />);
  view.rerender(<Overview projectId="p2" onCreate={() => {}} />);
  await act(async () => lateA({ items: [event('a1', 'histórico de A')], next_before: null }));
  emit([event('a1', 'ao vivo de A'), event('b1', 'ao vivo de B')]);
  const list = screen.getByTestId('agent-list');
  expect(list.textContent).toContain('ao vivo de B');
  expect(list.textContent).not.toContain('de A');
  expect(screen.getAllByRole('treeitem')).toHaveLength(1);
});
