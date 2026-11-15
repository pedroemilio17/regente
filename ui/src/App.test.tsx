import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { api } from './api/client';
import type { AgentEvent, AgentView, Snapshot } from './api/types';
import { MaestroChat } from './components/Chat';
import { Topbar } from './components/Topbar';
import { useStore } from './store';

vi.mock('./api/stream', () => ({ connectStream: () => () => {}, subscribeEvents: () => () => {} }));
vi.mock('./components/Sidebar', () => ({
  Sidebar: ({ onToggle }: { onToggle: () => void }) => <button type="button" onClick={onToggle}>menu</button>,
}));
vi.mock('./screens/Overview', () => ({ Overview: () => <p>visão</p> }));
vi.mock('./screens/Profiles', () => ({ Profiles: () => <p>perfis</p> }));

const agent = (id: string, name: string, is_orchestrator: boolean, cli = 'claude'): AgentView => ({
  id, project_id: 'p1', name, cli, role: '', is_orchestrator, parent_id: null,
  account_id: null, model: '', pet: {}, permissions: {}, settings: {}, budget_usd: null, paused: false, pause_reason: null, created_at: '',
  live: { status: 'idle', doing_now: '', task_id: null, recent_changes: [], cost_usd: 0, tokens: 0, last_ts: null, pause_reason: null },
} as AgentView);

const snapshot = (api_version: string, extra: Partial<Snapshot> = {}): Snapshot => ({
  type: 'snapshot', api_version, version: '0.1.0', last_seq: 0,
  projects: [], agents: [], tasks: [], accounts: [], usage: { generated_at: '', accounts: [], agents: [] }, approvals: [], messages: [],
  ...extra,
});

const team = () => snapshot('1', {
  projects: [{ id: 'p1', name: 'Loja' } as Snapshot['projects'][number]],
  agents: [agent('m1', 'Regente', true), agent('a2', 'Codex', false, 'codex')],
});

beforeEach(() => {
  localStorage.clear();
  window.location.hash = '';
  vi.spyOn(api, 'timeline').mockResolvedValue({ items: [], next_before: null });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useStore.setState({ ready: false, apiVersion: null, incompatibleVersion: null, selectedAgentId: null, panelTab: 'chat', panelAgentId: null, configOpen: false, chatRecipientId: null, connection: 'reconnecting', lastEventAt: {} });
});

describe('API compatibility warning', () => {
  it('warns for newer and older contracts and clears the warning after reconnecting compatibly', () => {
    render(<App />);
    // A future snapshot need not have the fields required by this UI.
    act(() => useStore.getState().applySnapshot({ type: 'snapshot', api_version: '2' } as Snapshot));
    expect(screen.getByRole('alert').textContent).toContain('interface 1, daemon 2');
    expect(useStore.getState().ready).toBe(false);
    act(() => useStore.getState().applySnapshot(snapshot('0')));
    expect(screen.getByRole('alert').textContent).toContain('interface 1, daemon 0');
    act(() => useStore.getState().applySnapshot(snapshot('1')));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('AppShell', () => {
  it('keeps the chat mounted while the sidebar toggles and the section changes', () => {
    render(<App />);
    act(() => useStore.getState().applySnapshot(team()));
    expect(screen.getByRole('heading', { name: 'Maestro · Claude Code' })).toBeTruthy();
    const box = screen.getByLabelText('Mensagem para Regente') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'rascunho' } });
    fireEvent.click(screen.getByText('menu'));
    act(() => { window.location.hash = '#/perfis'; window.dispatchEvent(new HashChangeEvent('hashchange')); });
    expect(screen.getByText('perfis')).toBeTruthy();
    expect(screen.getByLabelText('Mensagem para Regente')).toBe(box);
    expect(box.value).toBe('rascunho');
    expect(JSON.parse(localStorage.getItem('regente.layout.p1') ?? '{}').sidebarExpanded).toBe(true);
  });

  it('never changes the recipient when an agent is selected elsewhere', () => {
    render(<App />);
    act(() => useStore.getState().applySnapshot(team()));
    act(() => useStore.getState().setSelectedAgentId('a2'));
    act(() => { window.location.hash = '#/agente/a2'; window.dispatchEvent(new HashChangeEvent('hashchange')); });
    expect((screen.getByLabelText('Destinatário') as HTMLSelectElement).value).toBe('m1');
  });
});

describe('Fluxo B: inspecionar um agente sem perder a conversa', () => {
  it('popover, Atividade, Voltar ao chat com rascunho intacto e destinatário Maestro', async () => {
    render(<App />);
    act(() => useStore.getState().applySnapshot(team()));
    const box = screen.getByLabelText('Mensagem para Regente') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'rascunho' } });
    const recipient = () => (screen.getByLabelText('Destinatário') as HTMLSelectElement).value;

    act(() => { useStore.getState().setConnection('online'); useStore.getState().setSelectedAgentId('a2'); });
    const pop = screen.getByRole('dialog', { name: 'Inspetor de Codex' });
    expect(pop.textContent).toContain('Disponível');
    expect(recipient()).toBe('m1');

    fireEvent.click(screen.getByText('Abrir atividade'));
    expect(screen.getByRole('tab', { name: 'Atividade' }).getAttribute('aria-selected')).toBe('true');
    expect(await screen.findByRole('region', { name: 'Atividade de Codex' })).toBeTruthy();
    expect(screen.getByText('visão')).toBeTruthy();
    expect(recipient()).toBe('m1');

    fireEvent.click(screen.getByText('Voltar ao chat'));
    expect(screen.getByRole('tab', { name: 'Chat' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByLabelText('Mensagem para Regente')).toBe(box);
    expect(box.value).toBe('rascunho');
    expect(recipient()).toBe('m1');

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useStore.getState().selectedAgentId).toBeNull();
  });

  it('a rota #/agente/<id> abre a aba Atividade e mantém o canvas', async () => {
    render(<App />);
    act(() => useStore.getState().applySnapshot(team()));
    act(() => { window.location.hash = '#/agente/a2'; window.dispatchEvent(new HashChangeEvent('hashchange')); });
    expect(await screen.findByRole('region', { name: 'Atividade de Codex' })).toBeTruthy();
    expect(screen.getByText('visão')).toBeTruthy();
    expect(window.location.hash).toBe('#/projeto/p1');
  });

  it('só "Falar com este agente" muda o destinatário', () => {
    render(<App />);
    act(() => useStore.getState().applySnapshot(team()));
    act(() => useStore.getState().setSelectedAgentId('a2'));
    fireEvent.click(screen.getByText('Falar com este agente'));
    expect((screen.getByLabelText('Destinatário') as HTMLSelectElement).value).toBe('a2');
  });
});

describe('MaestroChat', () => {
  beforeEach(() => { act(() => useStore.getState().applySnapshot(team())); });

  it('keeps one draft per recipient across remounts and switches with @nome', () => {
    const first = render(<MaestroChat projectId="p1" />);
    fireEvent.change(screen.getByLabelText('Mensagem para Regente'), { target: { value: 'para o maestro' } });
    first.unmount();

    render(<MaestroChat projectId="p1" />);
    const box = screen.getByLabelText('Mensagem para Regente') as HTMLTextAreaElement;
    expect(box.value).toBe('para o maestro');

    fireEvent.change(box, { target: { value: '@codex revise' } });
    expect((screen.getByLabelText('Destinatário') as HTMLSelectElement).value).toBe('a2');
    expect((screen.getByLabelText('Mensagem para Codex') as HTMLTextAreaElement).value).toBe('revise');

    fireEvent.change(screen.getByLabelText('Destinatário'), { target: { value: 'm1' } });
    expect((screen.getByLabelText('Mensagem para Regente') as HTMLTextAreaElement).value).toBe('');
    fireEvent.change(screen.getByLabelText('Destinatário'), { target: { value: 'a2' } });
    expect((screen.getByLabelText('Mensagem para Codex') as HTMLTextAreaElement).value).toBe('revise');
  });

  it('sends to the Maestro by default', async () => {
    const send = vi.spyOn(api, 'messageAgent').mockResolvedValue({ ok: true });
    render(<MaestroChat projectId="p1" />);
    const box = screen.getByLabelText('Mensagem para Regente');
    fireEvent.change(box, { target: { value: 'Oi' } });
    await act(async () => { fireEvent.keyDown(box, { key: 'Enter' }); });
    expect(send).toHaveBeenCalledWith('m1', 'Oi');
    expect(localStorage.getItem('regente.draft.p1.m1')).toBeNull();
  });
});

class FakeSocket {
  static all: FakeSocket[] = [];
  static OPEN = 1;
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() { FakeSocket.all.push(this); }
  open() { this.readyState = 1; this.onopen?.(); }
  send(message: object) { this.onmessage?.({ data: JSON.stringify(message) }); }
  close() { this.readyState = 3; this.onclose?.(); }
}

describe('connection badge', () => {
  it('follows the socket and keeps the data when it drops', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('WebSocket', FakeSocket);
    vi.spyOn(api, 'health').mockResolvedValue({ api_version: '1' } as Awaited<ReturnType<typeof api.health>>);
    vi.spyOn(api, 'listTasks').mockResolvedValue([]);
    const { connectStream } = await vi.importActual<typeof import('./api/stream')>('./api/stream');
    render(<Topbar projectId="p1" chatOpen onToggleChat={() => {}} onSettings={() => {}} />);
    const badge = () => screen.getByTestId('connection').textContent;
    const stop = connectStream();
    try {
      const event: AgentEvent = { seq: 1, id: 'e1', ts: '2026-11-15T10:00:00Z', agent_id: 'a2', kind: 'thinking', summary: 'Lendo' };
      act(() => { FakeSocket.all[0]!.open(); FakeSocket.all[0]!.send(team()); FakeSocket.all[0]!.send({ type: 'events', items: [event] }); });
      expect(badge()).toBe('Conectado');
      expect(useStore.getState().lastEventAt.a2).toBe(event.ts);

      act(() => FakeSocket.all[0]!.close());
      expect(badge()).toBe('Reconectando…');
      expect(useStore.getState().projects).toHaveLength(1);
      expect(useStore.getState().agents.a2?.live.last_ts).toBe(event.ts);

      for (let i = 1; i <= 3; i += 1) {
        act(() => { vi.advanceTimersByTime(10_000); FakeSocket.all[i]!.close(); });
      }
      expect(badge()).toBe('Sem conexão');

      // Back online: the snapshot covers seq 1, so replaying it changes nothing.
      act(() => { vi.advanceTimersByTime(10_000); });
      const last = FakeSocket.all[FakeSocket.all.length - 1]!;
      const feed = useStore.getState().feed.length;
      act(() => { last.open(); last.send({ ...team(), last_seq: 1 }); last.send({ type: 'events', items: [event] }); vi.advanceTimersByTime(300); });
      expect(badge()).toBe('Conectado');
      expect(useStore.getState().lastSeq).toBe(1);
      expect(useStore.getState().feed).toHaveLength(feed);
    } finally {
      stop();
      vi.unstubAllGlobals();
      vi.useRealTimers();
    }
  });
});
