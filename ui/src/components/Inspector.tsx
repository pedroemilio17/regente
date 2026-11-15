// Quick inspector (popover over the canvas) and deep inspector (Atividade and
// Arquivos tabs of the right-hand panel). Parts come from the old agent screen.

import { lazy, Suspense, useEffect, useState, type FormEvent } from 'react';
import { agentPath, api } from '../api/client';
import type { AgentView, LatestDiff } from '../api/types';
import { useNow } from '../hooks/useNow';
import { formatAgo, formatTime, formatTokens, formatUsd } from '../lib/format';
import { cliInfo } from '../lib/labels';
import { AGENT_STATUS_LABEL, agentStatus, taskStatus, TASK_STATUS_LABEL } from '../lib/projection';
import { useStore, type PanelTab } from '../store';
import { useTimeline } from './Chat';
import { DiffView } from './DiffView';
import { ModelEffort } from './ModelEffort';
import { StatusPill } from './Status';
import { Timeline } from './Timeline';

// xterm.js lives in its own chunk, fetched only when the terminal is opened.
const TerminalPanel = lazy(() => import('./TerminalPanel'));

function errorText(cause: unknown, fallback: string): string {
  return cause instanceof Error && cause.message ? cause.message : fallback;
}

export function ResumeButton({ agentId, small = true }: { agentId: string; small?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const resume = async () => {
    setBusy(true);
    setError('');
    try {
      await api.resumeAgent(agentId);
    } catch (cause) {
      setError(errorText(cause, 'Não foi possível retomar'));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button type="button" className={`btn${small ? ' small' : ''}`} onClick={resume} disabled={busy}>
        Retomar
      </button>
      {error && <span className="error-text">{error}</span>}
    </>
  );
}

/** Popover over the canvas for the selected agent. Never changes the chat recipient by itself. */
export function QuickInspector({ agentId, onTalk }: { agentId: string; onTalk: () => void }) {
  const agent = useStore((state) => state.agents[agentId]);
  const tasks = useStore((state) => state.tasks);
  const connected = useStore((state) => state.connection === 'online' || state.connection === 'degraded');
  const close = () => useStore.getState().setSelectedAgentId(null);
  const now = useNow();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') useStore.getState().setSelectedAgentId(null);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);

  if (!agent) return null;
  const { status, currentTask, lastActivity } = agentStatus(agent, tasks, [], now, connected);
  const deps = (currentTask?.depends_on ?? []).map((id) => tasks.find((task) => task.id === id) ?? { id, title: id });
  const open = (config = false) => useStore.getState().openPanel('activity', agentId, config);

  return (
    <section className="inspector-pop card" role="dialog" aria-label={`Inspetor de ${agent.name}`}>
      <div className="card-head">
        <h2>{agent.name}</h2>
        <button type="button" className="btn small" onClick={close} aria-label="Fechar inspetor">
          ×
        </button>
      </div>
      <dl className="summary">
        <dt>Papel</dt>
        <dd>{agent.is_orchestrator ? 'orquestrador' : agent.role || '—'}</dd>
        <dt>Status</dt>
        <dd data-testid="inspector-status">{AGENT_STATUS_LABEL[status]}</dd>
        <dt>Tarefa</dt>
        <dd>{currentTask?.title ?? 'Nenhuma'}</dd>
        <dt>Último evento</dt>
        <dd>{lastActivity ? `${lastActivity.summary || '—'} · ${formatAgo(lastActivity.ts, now)}` : 'Nenhum'}</dd>
        <dt>Depende de</dt>
        <dd>
          {deps.length === 0
            ? 'Nada'
            : deps.map((task) => ('status' in task ? `${task.title} (${TASK_STATUS_LABEL[taskStatus(task)]})` : task.title)).join(', ')}
        </dd>
      </dl>
      <div className="head-actions">
        <button type="button" className="btn small" onClick={() => open()}>
          Abrir atividade
        </button>
        <button type="button" className="btn small" onClick={onTalk}>
          Falar com este agente
        </button>
        <button type="button" className="btn small" onClick={() => open(true)}>
          Configurar
        </button>
      </div>
    </section>
  );
}

/** Latest change with its diff; reloaded when the agent edits another file. */
function LatestChange({ agentId, latestEditId }: { agentId: string; latestEditId: string | undefined }) {
  const [latest, setLatest] = useState<LatestDiff | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.latestDiff(agentId).then(
      (value) => {
        if (!cancelled) setLatest(value);
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [agentId, latestEditId]);

  const event = latest?.event;
  return (
    <section className="card" aria-label="Última alteração" style={{ gap: 12 }}>
      <div className="card-head">
        <h2>Última alteração</h2>
        {event && (
          <span className="note mono">
            {event.target} · {formatTime(event.ts)}
          </span>
        )}
      </div>
      {!event && <p className="empty">Este agente ainda não alterou arquivos.</p>}
      {event && (
        <div style={{ fontSize: 13 }}>
          {event.summary}{' '}
          {(event.lines_added ?? 0) > 0 && <span className="plus">+{event.lines_added}</span>}{' '}
          {(event.lines_removed ?? 0) > 0 && <span className="minus">−{event.lines_removed}</span>}
        </div>
      )}
      {event && (latest?.diff ? <DiffView diff={latest.diff} /> : <p className="empty">Diff não disponível.</p>)}
    </section>
  );
}

/** Name, role, model and reasoning level; models come from the account's CLI. */
function AgentSettings({ agent }: { agent: AgentView }) {
  const [name, setName] = useState(agent.name);
  const [role, setRole] = useState(agent.role);
  const [model, setModel] = useState(agent.model);
  const [effort, setEffort] = useState(typeof agent.settings.effort === 'string' ? agent.settings.effort : '');
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);

  const save = async (event: FormEvent) => {
    event.preventDefault();
    setNote(null);
    try {
      const settings = { ...agent.settings };
      delete settings.effort;
      if (effort && agent.cli !== 'agy') settings.effort = effort;
      await api.patchAgent(agent.id, { name: name.trim(), role: role.trim(), model: model.trim(), settings });
      setNote({ ok: true, text: 'Configuração salva.' });
    } catch (cause) {
      setNote({ ok: false, text: errorText(cause, 'Não foi possível salvar.') });
    }
  };

  return (
    <form onSubmit={save} style={{ display: 'grid', gap: 12 }}>
      <div className="field">
        <label htmlFor="cfg-name">Nome</label>
        <input id="cfg-name" className="input" value={name} onChange={(event) => setName(event.target.value)} />
      </div>
      <div className="field">
        <label htmlFor="cfg-role">Papel</label>
        <input id="cfg-role" className="input" value={role} onChange={(event) => setRole(event.target.value)} />
      </div>
      <ModelEffort cli={agent.cli} accountId={agent.account_id} model={model} onModel={setModel} effort={effort} onEffort={setEffort} idPrefix="cfg" />
      <div>
        <button type="submit" className="btn primary">
          Salvar
        </button>{' '}
        {note && (
          <span className={note.ok ? 'ok-text' : 'error-text'} role="status">
            {note.text}
          </span>
        )}
      </div>
    </form>
  );
}

/** Settings, pause/resume, terminal and spend. */
function AgentConfig({ agent }: { agent: AgentView }) {
  const todayUsd = useStore((state) => state.usage?.agents.find((entry) => entry.agent_id === agent.id)?.windows.day?.cost_usd);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const { live } = agent;

  const pause = async () => {
    setBusy(true);
    setError('');
    try {
      await api.pauseAgent(agent.id);
    } catch (cause) {
      setError(errorText(cause, 'Não foi possível pausar o agente.'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="config-body">
      <AgentSettings key={`${agent.name}|${agent.role}|${agent.model}`} agent={agent} />
      <div className="head-actions">
        {live.status === 'paused' ? (
          <ResumeButton agentId={agent.id} small={false} />
        ) : (
          <button type="button" className="btn" onClick={pause} disabled={busy}>
            Pausar
          </button>
        )}
        <button type="button" className="btn" onClick={() => setTerminalOpen((open) => !open)}>
          {terminalOpen ? 'Fechar terminal' : 'Abrir terminal'}
        </button>
      </div>
      {error && (
        <span className="error-text" role="alert">
          {error}
        </span>
      )}
      {terminalOpen && (
        <section className="terminal-card" aria-label="Terminal">
          <Suspense fallback={<p className="empty">Carregando terminal…</p>}>
            <TerminalPanel path={agentPath(agent.id, '/pty')} />
          </Suspense>
        </section>
      )}
      <section className="card" aria-label="Gasto">
        <h2>Gasto</h2>
        <div className="stat-row">
          <div className="stat">
            <span className="label">Total</span>
            <b data-testid="agent-cost">{formatUsd(live.cost_usd)}</b>
          </div>
          <div className="stat">
            <span className="label">Hoje</span>
            <b>{formatUsd(todayUsd ?? 0)}</b>
          </div>
        </div>
        <div className="summary">
          <dt>Tokens</dt>
          <dd>{formatTokens(live.tokens)}</dd>
          <dt>Orçamento</dt>
          <dd>{agent.budget_usd != null ? `${formatUsd(agent.budget_usd)} por dia` : 'sem limite'}</dd>
        </div>
        {agent.budget_usd != null && agent.budget_usd > 0 && (
          <span className="bar tall tone-codex">
            <span style={{ width: `${Math.min(100, ((todayUsd ?? 0) / agent.budget_usd) * 100)}%` }} />
          </span>
        )}
        <p className="empty" style={{ fontSize: 12 }}>
          {agent.cli === 'claude' ? 'Custo informado pelo Claude Code.' : 'Custo estimado pelos tokens e pela tabela de preços do modelo.'}
        </p>
      </section>
    </div>
  );
}

const FILE_KINDS = new Set(['file_edit', 'file_read']);

/** Atividade and Arquivos tabs for one agent. Mount with `key={agentId}`. */
export function AgentInspector({ agentId, tab }: { agentId: string; tab: Exclude<PanelTab, 'chat'> }) {
  const agent = useStore((state) => state.agents[agentId]);
  const configOpen = useStore((state) => state.configOpen);
  const timeline = useTimeline(agentId);
  const back = () => useStore.getState().openPanel('chat');

  if (!agent) return <p className="empty">Agente não encontrado.</p>;
  const { live } = agent;
  const files = timeline.events.filter((event) => FILE_KINDS.has(event.kind));

  return (
    <section className="inspector" aria-label={`${tab === 'activity' ? 'Atividade' : 'Arquivos'} de ${agent.name}`}>
      <header className="inspector-head">
        <div>
          <h2>{agent.name}</h2>
          <span className="note">
            {cliInfo(agent.cli).label} · {agent.is_orchestrator ? 'orquestrador' : agent.role}
          </span>
        </div>
        <StatusPill status={live.status} />
        <button type="button" className="btn small" onClick={back}>
          Voltar ao chat
        </button>
      </header>
      {timeline.error && <span className="error-text">{timeline.error}</span>}
      {tab === 'activity' ? (
        <>
          {live.status === 'paused' && (
            <div className="pause-box" role="status" data-testid="pause-box">
              <span className="reason">
                <b>Agente pausado.</b> {live.pause_reason || agent.pause_reason || ''}
              </span>
            </div>
          )}
          <details
            className="card config-panel"
            open={configOpen}
            onToggle={(event) => useStore.getState().setConfigOpen(event.currentTarget.open)}
          >
            <summary>Configuração</summary>
            {configOpen && <AgentConfig agent={agent} />}
          </details>
          <Timeline events={timeline.events} hasMore={timeline.hasMore} loading={timeline.loading} onLoadMore={timeline.loadMore} />
        </>
      ) : (
        <>
          <LatestChange agentId={agentId} latestEditId={live.recent_changes[0]?.event_id} />
          <Timeline events={files} hasMore={timeline.hasMore} loading={timeline.loading} onLoadMore={timeline.loadMore} />
        </>
      )}
    </section>
  );
}
