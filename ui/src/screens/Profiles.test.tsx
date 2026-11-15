import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, cleanup, within } from '@testing-library/react';
import { Profiles } from './Profiles';
import { useStore } from '../store';
import { api } from '../api/client';
import type { Account, AccountStatus } from '../api/types';

// xterm.js does not run in jsdom; the stub shows which PTY route was opened.
vi.mock('../components/TerminalPanel', () => ({
  default: ({ path }: { path: string }) => <div data-testid="terminal">{path}</div>,
}));

const account = (provider: 'claude' | 'codex' | 'agy'): Account => ({
  id: `acc_${provider}`,
  provider,
  login_type: 'subscription',
  cli_path: provider,
  default_model: '',
  limit_rule: {},
  max_concurrent: 1,
});

const STATUSES: Record<string, AccountStatus> = {
  acc_claude: { state: 'connected' },
  acc_codex: { state: 'disconnected' },
  acc_agy: { state: 'unknown', detail: 'CLI não encontrado no PATH.' },
};

describe('Profiles: login simples', () => {
  beforeEach(() => {
    useStore.setState({
      accounts: [account('claude'), account('codex'), account('agy')],
      usage: { generated_at: new Date().toISOString(), accounts: [], agents: [] },
    });
    vi.spyOn(api, 'accountStatus').mockImplementation((id) => Promise.resolve(STATUSES[id] ?? { state: 'unknown' }));
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('mostra o status de cada conta e um único botão de login', async () => {
    render(<Profiles />);
    const claude = screen.getByLabelText('Conta Claude');
    expect(await within(claude).findByText('Conectado')).not.toBeNull();
    expect(within(claude).getByRole('button', { name: 'Reconectar' })).not.toBeNull();

    const codex = screen.getByLabelText('Conta ChatGPT');
    expect(await within(codex).findByText('Não conectado')).not.toBeNull();
    expect(within(codex).getByRole('button', { name: 'Entrar' })).not.toBeNull();

    const agy = screen.getByLabelText('Conta Antigravity');
    expect(await within(agy).findByText('Desconhecido')).not.toBeNull();
    expect(within(agy).getByTestId('account-status-detail').textContent).toBe('CLI não encontrado no PATH.');
  });

  it('avisa quando a consulta de status falha, em vez de só mostrar Desconhecido', async () => {
    vi.mocked(api.accountStatus).mockImplementation((id) =>
      id === 'acc_codex' ? Promise.reject(new Error('HTTP 404')) : Promise.resolve(STATUSES[id] ?? { state: 'unknown' }),
    );
    render(<Profiles />);
    const codex = screen.getByLabelText('Conta ChatGPT');
    expect(
      (await within(codex).findByTestId('account-status-detail')).textContent,
    ).toBe('Não foi possível consultar o login. Reinicie o Regente para atualizar o daemon.');

    const agy = screen.getByLabelText('Conta Antigravity');
    expect((await within(agy).findByTestId('account-status-detail')).textContent).toBe('CLI não encontrado no PATH.');
  });

  it('abre o terminal de login e consulta o status de novo ao fechar', async () => {
    render(<Profiles />);
    const codex = screen.getByLabelText('Conta ChatGPT');
    fireEvent.click(await within(codex).findByRole('button', { name: 'Entrar' }));
    expect((await within(codex).findByTestId('terminal')).textContent).toBe('/accounts/acc_codex/login');

    STATUSES.acc_codex = { state: 'connected' };
    fireEvent.click(within(codex).getByRole('button', { name: 'Fechar login' }));
    expect(await within(codex).findByText('Conectado')).not.toBeNull();
    expect(within(codex).queryByTestId('terminal')).toBeNull();
    expect(api.accountStatus).toHaveBeenCalledWith('acc_codex');
    STATUSES.acc_codex = { state: 'disconnected' };
  });

  it('guarda tipo de login, CLI, modelo e limites no bloco Avançado', () => {
    render(<Profiles />);
    const claude = screen.getByLabelText('Conta Claude');
    const advanced = claude.querySelector('details') as HTMLDetailsElement;
    expect(advanced.open).toBe(false);
    expect(within(advanced).getByText('Avançado')).not.toBeNull();
    expect(within(advanced).getByText('Tipo de login')).not.toBeNull();
    expect(within(advanced).getByText('Caminho do CLI')).not.toBeNull();
    expect(within(advanced).getByRole('button', { name: 'Salvar' })).not.toBeNull();

    fireEvent.change(within(advanced).getByRole('combobox'), { target: { value: 'api_key' } });
    expect(within(advanced).getByText('ANTHROPIC_API_KEY')).not.toBeNull();
  });
});
