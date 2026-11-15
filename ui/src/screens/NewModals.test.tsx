import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError } from '../api/client';
import type { Account, AgentView, CreatedProject, Project } from '../api/types';
import { useStore } from '../store';
import { NewAgent } from './NewAgent';
import { NewProject } from './NewProject';

beforeEach(() => {
  useStore.setState({ projects: [{ id: 'p1', name: 'Projeto' } as Project], refresh: async () => {} });
  window.location.hash = '';
});
afterEach(cleanup);

describe('Novo projeto', () => {
  it('opens focused on the name, sends only name, root and context, and navigates to the project', async () => {
    const create = vi.spyOn(api, 'createProject').mockResolvedValue({ project: { id: 'p9' } } as CreatedProject);
    const onClose = vi.fn();
    render(<NewProject onClose={onClose} />);
    expect(screen.getByRole('dialog', { name: 'Novo projeto' }).getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(screen.getByLabelText('Nome'));
    fireEvent.change(screen.getByLabelText('Nome'), { target: { value: ' Loja ' } });
    fireEvent.change(screen.getByLabelText('Pasta (caminho absoluto)'), { target: { value: '/tmp/loja' } });
    fireEvent.change(screen.getByLabelText('Descrição (opcional)'), { target: { value: 'Loja virtual' } });
    fireEvent.click(screen.getByRole('button', { name: 'Criar projeto' }));
    await waitFor(() => expect(window.location.hash).toBe('#/projeto/p9'));
    expect(create).toHaveBeenCalledWith({ name: 'Loja', root: '/tmp/loja', context: 'Loja virtual' });
    expect(onClose).toHaveBeenCalled();
  });

  it('edits an existing project: prefilled, saves name, folder and description, shows the daemon refusal', async () => {
    const save = vi.spyOn(api, 'patchProject')
      .mockRejectedValueOnce(new ApiError(400, 'conclua ou cancele as tarefas ativas antes de trocar a pasta'))
      .mockResolvedValueOnce({ id: 'p1' } as Project);
    const onClose = vi.fn();
    const project = { id: 'p1', name: 'Projeto', root: '/tmp/a', context: 'Antiga' } as Project;
    render(<NewProject project={project} onClose={onClose} />);
    expect(screen.getByRole('dialog', { name: 'Configurações do projeto' })).toBeTruthy();
    expect((screen.getByLabelText('Descrição (opcional)') as HTMLTextAreaElement).value).toBe('Antiga');
    fireEvent.change(screen.getByLabelText('Pasta (caminho absoluto)'), { target: { value: '/tmp/b' } });
    fireEvent.change(screen.getByLabelText('Descrição (opcional)'), { target: { value: 'Nova' } });
    fireEvent.click(screen.getByRole('button', { name: 'Salvar' }));
    expect((await screen.findByRole('alert')).textContent).toContain('tarefas ativas');
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Salvar' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(save).toHaveBeenLastCalledWith('p1', { name: 'Projeto', root: '/tmp/b', context: 'Nova' });
    expect(window.location.hash).toBe('');
  });

  it('closes on Esc and gives focus back to the trigger', () => {
    const trigger = document.createElement('button');
    document.body.append(trigger);
    trigger.focus();
    const onClose = vi.fn();
    const { unmount } = render(<NewProject onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
    unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });
});

describe('Escolher pasta', () => {
  const pick = () => fireEvent.click(screen.getByRole('button', { name: 'Escolher pasta no explorador de arquivos' }));

  it('fills the path and suggests the folder name when the name is empty', async () => {
    let finish: (value: { path: string | null }) => void = () => {};
    vi.spyOn(api, 'pickFolder').mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    render(<NewProject onClose={() => {}} />);
    pick();
    expect((screen.getByRole('button', { name: 'Escolher pasta no explorador de arquivos' }) as HTMLButtonElement).disabled).toBe(true);
    finish({ path: '/home/ana/loja-virtual' });
    await waitFor(() => expect((screen.getByLabelText('Pasta (caminho absoluto)') as HTMLInputElement).value).toBe('/home/ana/loja-virtual'));
    expect((screen.getByLabelText('Nome') as HTMLInputElement).value).toBe('loja-virtual');
    expect((screen.getByRole('button', { name: 'Escolher pasta no explorador de arquivos' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('keeps a name already typed', async () => {
    vi.spyOn(api, 'pickFolder').mockResolvedValue({ path: '/srv/app' });
    render(<NewProject onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Nome'), { target: { value: 'Meu app' } });
    pick();
    await waitFor(() => expect((screen.getByLabelText('Pasta (caminho absoluto)') as HTMLInputElement).value).toBe('/srv/app'));
    expect((screen.getByLabelText('Nome') as HTMLInputElement).value).toBe('Meu app');
  });

  it('changes nothing when cancelled', async () => {
    const picked = vi.spyOn(api, 'pickFolder').mockResolvedValue({ path: null });
    render(<NewProject onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Pasta (caminho absoluto)'), { target: { value: '/tmp/antes' } });
    pick();
    await waitFor(() => expect(picked).toHaveBeenCalled());
    await waitFor(() => expect((screen.getByRole('button', { name: 'Escolher pasta no explorador de arquivos' }) as HTMLButtonElement).disabled).toBe(false));
    expect((screen.getByLabelText('Pasta (caminho absoluto)') as HTMLInputElement).value).toBe('/tmp/antes');
    expect((screen.getByLabelText('Nome') as HTMLInputElement).value).toBe('');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the daemon error', async () => {
    vi.spyOn(api, 'pickFolder').mockRejectedValue(new ApiError(503, 'Não foi possível abrir o seletor de pastas; digite o caminho.'));
    render(<NewProject onClose={() => {}} />);
    pick();
    expect((await screen.findByRole('alert')).textContent).toBe('Não foi possível abrir o seletor de pastas; digite o caminho.');
  });
});

describe('Novo agente', () => {
  it('sends only cli, role flag and the current project', async () => {
    const create = vi.spyOn(api, 'createAgent').mockResolvedValue({} as AgentView);
    render(<NewAgent projectId="p2" onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'codex' } });
    fireEvent.change(screen.getByLabelText('Função'), { target: { value: 'orchestrator' } });
    fireEvent.click(screen.getByRole('button', { name: 'Criar agente' }));
    await waitFor(() => expect(create).toHaveBeenCalledWith({ project_id: 'p2', cli: 'codex', is_orchestrator: true }));
  });

  it('uses the first project and the basic scenario for the fake engine', async () => {
    const create = vi.spyOn(api, 'createAgent').mockResolvedValue({} as AgentView);
    render(<NewAgent onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'fake' } });
    fireEvent.click(screen.getByRole('button', { name: 'Criar agente' }));
    await waitFor(() => expect(create).toHaveBeenCalledWith({ project_id: 'p1', cli: 'fake', is_orchestrator: false, settings: { fake_scenario: 'basic' } }));
  });

  it('sends model only when chosen and resets it when the engine changes', async () => {
    const create = vi.spyOn(api, 'createAgent').mockResolvedValue({} as AgentView);
    render(<NewAgent projectId="p2" onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Modelo'), { target: { value: 'opus' } });
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'codex' } });
    expect((screen.getByLabelText('Modelo') as HTMLSelectElement).value).toBe('');
    fireEvent.click(screen.getByRole('button', { name: 'Criar agente' }));
    await waitFor(() => expect(create).toHaveBeenLastCalledWith({ project_id: 'p2', cli: 'codex', is_orchestrator: false }));
  });

  it('sends a free-text model from Outro', async () => {
    const create = vi.spyOn(api, 'createAgent').mockResolvedValue({} as AgentView);
    render(<NewAgent projectId="p2" onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'codex' } });
    fireEvent.change(screen.getByLabelText('Modelo'), { target: { value: '__other' } });
    fireEvent.change(screen.getByLabelText('Nome do modelo'), { target: { value: ' gpt-x ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Criar agente' }));
    await waitFor(() => expect(create).toHaveBeenLastCalledWith({ project_id: 'p2', cli: 'codex', is_orchestrator: false, model: 'gpt-x' }));
  });

  it('lists the models of the account, offers reasoning levels and sends effort', async () => {
    useStore.setState({ accounts: [{ id: 'acc_codex', provider: 'codex' } as Account] });
    vi.spyOn(api, 'accountModels').mockResolvedValue({ models: [{ id: 'gpt-x', label: 'GPT X', efforts: ['low', 'ultra'] }] });
    const create = vi.spyOn(api, 'createAgent').mockResolvedValue({} as AgentView);
    render(<NewAgent projectId="p2" onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'codex' } });
    await screen.findByRole('option', { name: 'GPT X (gpt-x)' });
    fireEvent.change(screen.getByLabelText('Modelo'), { target: { value: 'gpt-x' } });
    fireEvent.change(screen.getByLabelText('Raciocínio'), { target: { value: 'ultra' } });
    fireEvent.click(screen.getByRole('button', { name: 'Criar agente' }));
    await waitFor(() => expect(create).toHaveBeenLastCalledWith({ project_id: 'p2', cli: 'codex', is_orchestrator: false, model: 'gpt-x', settings: { effort: 'ultra' } }));
    useStore.setState({ accounts: [] });
  });

  it('agy has no separate reasoning field', () => {
    render(<NewAgent projectId="p2" onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Motor'), { target: { value: 'agy' } });
    expect(screen.queryByLabelText('Raciocínio')).toBeNull();
  });
});
