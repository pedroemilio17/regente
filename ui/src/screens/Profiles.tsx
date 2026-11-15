import { lazy, Suspense, useCallback, useEffect, useState, type FormEvent } from 'react';
import { accountPath, api } from '../api/client';
import type { Account, AccountStatus, UsageWindows } from '../api/types';
import { formatUsd, usageBars } from '../lib/format';
import { cliInfo, LOGIN_TYPES } from '../lib/labels';
import { ACCOUNT_STATES, API_KEY_ENV } from '../lib/auth';
import { useStore } from '../store';

// xterm.js lives in its own chunk, fetched only when the login terminal opens.
const TerminalPanel = lazy(() => import('../components/TerminalPanel'));

const WINDOW_TITLES = { '5h': 'Janela de 5h', day: 'Hoje', week: 'Semana' } as const;
const STATUS_FAILED = 'Não foi possível consultar o login. Reinicie o Regente para atualizar o daemon.';

function AccountCard({ account, windows }: { account: Account; windows: UsageWindows | undefined }) {
  const cli = cliInfo(account.provider);
  const [loginType, setLoginType] = useState(account.login_type);
  const [cliPath, setCliPath] = useState(account.cli_path);
  const [model, setModel] = useState(account.default_model);
  const [maxConcurrent, setMaxConcurrent] = useState(String(account.max_concurrent));
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [statusFailed, setStatusFailed] = useState(false);
  const [loginOpen, setLoginOpen] = useState(false);

  const refreshStatus = useCallback(() => {
    setStatus(null);
    setStatusFailed(false);
    api
      .accountStatus(account.id)
      .then(setStatus)
      .catch(() => {
        // Usually an older daemon without the status route: say so instead of a bare "Desconhecido".
        setStatusFailed(true);
        setStatus({ state: 'unknown', detail: STATUS_FAILED });
      });
  }, [account.id]);

  useEffect(refreshStatus, [refreshStatus]);

  const closeLogin = () => {
    setLoginOpen(false);
    refreshStatus();
  };

  const save = async (event: FormEvent) => {
    event.preventDefault();
    const max = Number(maxConcurrent);
    if (!Number.isInteger(max) || max < 1) {
      setNote({ ok: false, text: 'O máximo de agentes simultâneos precisa ser pelo menos 1.' });
      return;
    }
    setBusy(true);
    setNote(null);
    try {
      const updated = await api.patchAccount(account.id, {
        login_type: loginType,
        cli_path: cliPath.trim(),
        default_model: model.trim(),
        max_concurrent: max,
      });
      useStore.getState().upsertAccount(updated);
      setNote({ ok: true, text: 'Alterações salvas.' });
    } catch (cause) {
      setNote({ ok: false, text: cause instanceof Error ? cause.message : 'Não foi possível salvar.' });
    } finally {
      setBusy(false);
    }
  };

  const loginOptions = Object.keys(LOGIN_TYPES).includes(loginType)
    ? Object.keys(LOGIN_TYPES)
    : [loginType, ...Object.keys(LOGIN_TYPES)];

  return (
    <form
      className={`card tone-${cli.key}`}
      onSubmit={save}
      aria-label={`Conta ${cli.provider}`}
      data-testid="account-card"
      data-account={account.id}
    >
      <div className="account-head">
        <span className="logo" />
        <span className="names">
          <b>{cli.provider}</b>
          <small>{cli.label}</small>
        </span>
        <span
          className={`pill st-${status ? ACCOUNT_STATES[status.state].tone : 'idle'}`}
          data-testid="account-status"
          title={status?.detail}
        >
          <span className="dot" />
          {status ? ACCOUNT_STATES[status.state].label : 'Verificando…'}
        </span>
      </div>

      {status?.detail && (
        <small
          className={statusFailed ? 'error-text' : 'muted'}
          style={{ display: 'block', fontSize: 12 }}
          data-testid="account-status-detail"
        >
          {status.detail}
        </small>
      )}

      <div className="form-actions" style={{ justifyContent: 'flex-start' }}>
        <button
          type="button"
          className="btn small primary"
          onClick={() => (loginOpen ? closeLogin() : setLoginOpen(true))}
          aria-expanded={loginOpen}
        >
          {loginOpen ? 'Fechar login' : status?.state === 'connected' ? 'Reconectar' : 'Entrar'}
        </button>
        <span className="muted" style={{ fontSize: 11 }}>
          Gasto hoje <b style={{ color: 'var(--text)' }}>{formatUsd(windows?.day?.cost_usd)}</b>
        </span>
      </div>

      {loginOpen && (
        <section className="terminal-card" aria-label={`Login ${cli.provider}`}>
          <small className="muted" style={{ display: 'block', marginBottom: 6 }}>
            O login do {cli.provider} abre o navegador. Depois de concluir, feche o terminal.
          </small>
          <Suspense fallback={<p className="empty">Carregando terminal…</p>}>
            <TerminalPanel
              path={accountPath(account.id, '/login')}
              failMessage="Não foi possível abrir o terminal de login."
            />
          </Suspense>
        </section>
      )}

      {usageBars(windows).map((bar) => (
        <div className="window-row" key={bar.key} data-testid={`window-${bar.key}`}>
          <div className="line">
            <span>{WINDOW_TITLES[bar.key]}</span>
            <span>
              <b>{bar.value}</b>
              {bar.reset && ` · ${bar.reset}`}
            </span>
          </div>
          <span className="bar tall">{bar.width !== null && <span style={{ width: `${bar.width}%` }} />}</span>
        </div>
      ))}

      <details>
        <summary>Avançado</summary>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 12 }}>
          <div className="grid-2">
            <label className="field">
              <span>Tipo de login</span>
              <select className="select" value={loginType} onChange={(event) => { const value = event.target.value; if (value === 'subscription' || value === 'api_key') setLoginType(value); }}>
                {loginOptions.map((option) => (
                  <option key={option} value={option}>
                    {LOGIN_TYPES[option] ?? option}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>Máx. de agentes simultâneos</span>
              <input
                className="input"
                type="number"
                min={1}
                value={maxConcurrent}
                onChange={(event) => setMaxConcurrent(event.target.value)}
              />
            </label>
          </div>

          {loginType === 'api_key' && API_KEY_ENV[account.provider] && (
            <small className="muted" style={{ fontSize: 12 }}>
              Com chave de API, defina <code className="mono">{API_KEY_ENV[account.provider]}</code> no ambiente do
              daemon.
            </small>
          )}
          <label className="field">
            <span>Caminho do CLI</span>
            <input className="input mono" value={cliPath} onChange={(event) => setCliPath(event.target.value)} />
          </label>
          <label className="field">
            <span>Modelo padrão</span>
            <input
              className="input"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder="Padrão da conta"
            />
          </label>

          <div className="form-actions" style={{ justifyContent: 'flex-start' }}>
            <button type="submit" className="btn small" disabled={busy}>
              Salvar
            </button>
            {note && (
              <span className={note.ok ? 'ok-text' : 'error-text'} role="status">
                {note.text}
              </span>
            )}
          </div>
        </div>
      </details>
    </form>
  );
}

export function Profiles() {
  const accounts = useStore((state) => state.accounts);
  const usage = useStore((state) => state.usage);
  return (
    <>
      <header className="page-head">
        <div>
          <div className="crumb">Perfil</div>
          <h1>Contas e perfis</h1>
          <p className="lead">Entre com a assinatura de cada CLI e acompanhe os limites de cada plano.</p>
        </div>
      </header>
      {accounts.length === 0 && <p className="empty">Nenhuma conta encontrada.</p>}
      <div className="account-grid">
        {accounts.map((account) => (
          <AccountCard
            key={account.id}
            account={account}
            windows={usage?.accounts.find((entry) => entry.account_id === account.id)?.windows}
          />
        ))}
      </div>
      <p className="empty" style={{ fontSize: 12 }}>
        O percentual vem do próprio CLI: Claude Code (janelas de 5 horas e semanal) e Codex (último uso registrado).
        O Antigravity CLI não informa cota.
      </p>
    </>
  );
}
