import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AgentView, Task } from '../api/types';
import { useStore } from '../store';
import { QuickInspector } from './Inspector';

const agent = {
  id: 'a1', project_id: 'p1', name: 'Ana', cli: 'claude', role: 'dev', is_orchestrator: false, parent_id: null,
  account_id: null, model: '', pet: {}, permissions: {}, settings: {}, budget_usd: null, paused: false, pause_reason: null, created_at: '',
  live: { status: 'working', doing_now: 'Editando', task_id: 't2', recent_changes: [], cost_usd: 0, tokens: 0, last_ts: new Date().toISOString(), pause_reason: null },
} as AgentView;

const task = (id: string, title: string, status: Task['status'], depends_on: string[] = []) =>
  ({ id, project_id: 'p1', owner_agent_id: 'a1', title, status, depends_on } as Task);

describe('QuickInspector', () => {
  beforeEach(() => {
    useStore.setState({
      agents: { a1: agent }, connection: 'online', selectedAgentId: 'a1', panelTab: 'chat', panelAgentId: null, configOpen: false,
      tasks: [task('t1', 'Criar API', 'running'), task('t2', 'Tela de login', 'running', ['t1'])],
    });
  });
  afterEach(cleanup);

  it('mostra status, tarefa, último evento e dependências', () => {
    render(<QuickInspector agentId="a1" onTalk={() => {}} />);
    expect(screen.getByTestId('inspector-status').textContent).toBe('Trabalhando');
    expect(screen.getByText('Tela de login')).toBeTruthy();
    expect(screen.getByText(/^Editando/)).toBeTruthy();
    expect(screen.getByText('Criar API (Em andamento)')).toBeTruthy();
  });

  it('Configurar abre a Atividade com a configuração aberta; X limpa a seleção', () => {
    render(<QuickInspector agentId="a1" onTalk={() => {}} />);
    fireEvent.click(screen.getByText('Configurar'));
    expect(useStore.getState()).toMatchObject({ panelTab: 'activity', panelAgentId: 'a1', configOpen: true });
    act(() => fireEvent.click(screen.getByLabelText('Fechar inspetor')));
    expect(useStore.getState().selectedAgentId).toBeNull();
  });
});
