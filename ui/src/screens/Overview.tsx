import { memo, useMemo, useState, type FormEvent } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { api } from '../api/client';
import type { Alert, FeedItem } from '../api/reducer';
import type { AgentView, Approval, Project, Task } from '../api/types';
import { Canvas } from '../canvas/Canvas';
import { Pet } from '../components/Pet';
import { StatusDot } from '../components/Status';
import { formatTime, formatUsd, parseUsd } from '../lib/format';
import { ACTION_LABELS, cliInfo, isActive, statusStyle, TASK_STATUS } from '../lib/labels';
import { agentColor } from '../lib/pets';
import type { CreateKind } from '../App';
import { useStore } from '../store';

const FEED_SHORT = 6;

function errorText(cause: unknown, fallback: string): string {
  return cause instanceof Error && cause.message ? cause.message : fallback;
}

function NewTaskBox({ project, agentIds }: { project: Project; agentIds: string[] }) {
  const hasAgents = agentIds.length > 0;
  const agents = useStore((state) => state.agents);
  const [target, setTarget] = useState('');
  const agentId = agentIds.includes(target) ? target : agentIds[0];
  const [title, setTitle] = useState('');
  const [budget, setBudget] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [sent, setSent] = useState('');

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const clean = title.trim();
    if (!clean) {
      setError('Descreva a tarefa antes de enviar.');
      return;
    }
    const budgetUsd = parseUsd(budget);
    if (budget.trim() !== '' && budgetUsd === null) {
      setError('O orçamento precisa ser um valor em dólares, como 2,00.');
      return;
    }
    setBusy(true);
    setError('');
    setSent('');
    try {
      const task = await api.createTask({ project_id: project.id, title: clean, budget_usd: budgetUsd, agent_id: agentId });
      setTitle('');
      setBudget('');
      setSent(`Tarefa enviada: ${task.title}`);
      api.listTasks().then(useStore.getState().setTasks, () => undefined);
    } catch (cause) {
      setError(errorText(cause, 'Não foi possível enviar a tarefa.'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="card" onSubmit={submit} aria-label="Nova tarefa">
      <div className="task-box">
        <label className="field">
          <span>Para</span>
          <select className="select" value={agentId ?? ''} onChange={(event) => setTarget(event.target.value)} disabled={!hasAgents}>
            {agentIds.map((id) => {
              const agent = agents[id];
              return (
                <option key={id} value={id}>
                  {agent ? `${agent.name} · ${cliInfo(agent.cli).label} · ${statusStyle(agent.live.status).label}` : id}
                </option>
              );
            })}
          </select>
        </label>
        <label className="field title-field">
          <span>Nova tarefa</span>
          <input
            className="input"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder="Descreva o que o time deve fazer"
            disabled={!hasAgents}
          />
        </label>
        <label className="field budget-field">
          <span>Orçamento (US$)</span>
          <input
            className="input"
            value={budget}
            onChange={(event) => setBudget(event.target.value)}
            placeholder="opcional"
            inputMode="decimal"
            disabled={!hasAgents}
          />
        </label>
        <button type="submit" className="btn primary" disabled={busy || !hasAgents}>
          Enviar tarefa
        </button>
      </div>
      {!hasAgents && <span className="empty">Crie um agente neste projeto para poder enviar tarefas.</span>}
      {error && (
        <span className="error-text" role="alert">
          {error}
        </span>
      )}
      {sent && (
        <span className="ok-text" role="status">
          {sent}
        </span>
      )}
    </form>
  );
}

function stepMark(status: string): string {
  if (status === 'done') {
    return 'done';
  }
  if (status === 'running' || status === 'waiting') {
    return 'running';
  }
  return status === 'blocked' || status === 'cancelled' ? 'blocked' : '';
}

const CurrentTask = memo(function CurrentTask({ projectId }: { projectId: string }) {
  const tasks = useStore((state) => state.tasks);
  const agents = useStore((state) => state.agents);
  const { root, steps } = useMemo(() => {
    const roots = tasks.filter((task) => task.project_id === projectId && !task.parent_task_id);
    const byRecent = [...roots].sort((a, b) => b.created_at.localeCompare(a.created_at));
    const open = byRecent.find((task) => task.status !== 'done' && task.status !== 'cancelled');
    const chosen: Task | undefined = open ?? byRecent[0];
    return {
      root: chosen,
      steps: chosen ? tasks.filter((task) => task.root_task_id === chosen.id && task.id !== chosen.id) : [],
    };
  }, [tasks, projectId]);

  if (!root) {
    return null;
  }
  const owner = agents[root.owner_agent_id];
  const finished = root.status === 'done' || root.status === 'cancelled';
  const stateLabel =
    root.status === 'done'
      ? `Tarefa concluída · ${formatTime(root.finished_at)}`
      : root.status === 'cancelled'
        ? 'Tarefa cancelada'
        : root.status === 'blocked'
          ? 'Tarefa bloqueada'
          : `Tarefa ${root.status === 'queued' ? 'na fila' : 'em andamento'} · iniciada às ${formatTime(root.created_at)}`;

  return (
    <section className="card current-task" aria-label="Tarefa atual" data-testid="current-task">
      <div className="pet-frame" style={{ width: 84, height: 84, borderRadius: 14 }}>
        <Pet pet={owner?.pet} agent={owner} status={owner?.live.status} size={60} detailed />
      </div>
      <div className="body">
        <div className="state">
          <StatusDot status={finished ? 'idle' : root.status === 'blocked' ? 'paused' : 'working'} />
          {stateLabel}
          {root.budget_usd != null && <span>· orçamento {formatUsd(root.budget_usd)}</span>}
        </div>
        <h2>{root.title}</h2>
        {(root.summary || root.brief) && <p>{root.summary || root.brief}</p>}
      </div>
      {steps.length > 0 && (
        <ol className="steps">
          {steps.map((step, index) => (
            <li key={step.id}>
              <span className={`mark ${stepMark(step.status)}`}>{step.status === 'done' ? '✓' : index + 1}</span>
              <span className="text">
                <b title={step.title}>{step.title}</b>
                <small>
                  {agents[step.owner_agent_id]?.name ?? 'Agente'} · {TASK_STATUS[step.status] ?? step.status}
                </small>
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
});

function AlertRow({ alert, agentName }: { alert: Alert; agentName: string }) {
  const dismiss = useStore((state) => state.dismissAlert);
  return (
    <div className={`alert${alert.critical ? ' critical' : ''}`} role="alert" data-testid="security-alert">
      <div className="body">
        <span className="level">
          {alert.critical ? 'Alerta crítico de segurança' : 'Alerta de segurança'} · {agentName} · {formatTime(alert.ts)}
        </span>
        <span>{alert.summary}</span>
      </div>
      <a className="btn small" href={`#/agente/${encodeURIComponent(alert.agentId)}`}>
        Ver agente
      </a>
      <button type="button" className="btn small" onClick={() => dismiss(alert.id)}>
        Dispensar
      </button>
    </div>
  );
}

function ApprovalRow({ approval, agentName }: { approval: Approval; agentName: string }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const decide = async (decision: 'approve' | 'deny') => {
    setBusy(true);
    setError('');
    try {
      useStore.getState().applyApproval(await api.decideApproval(approval.id, decision));
    } catch (cause) {
      setError(errorText(cause, 'Não foi possível registrar a decisão.'));
      setBusy(false);
    }
  };
  return (
    <div className="approval" data-testid="approval">
      <div>
        <b>{agentName}</b> <span className="muted">pede aprovação · {ACTION_LABELS[approval.action] ?? approval.action}</span>
      </div>
      <div className="detail">{approval.detail}</div>
      <div className="actions">
        <button type="button" className="btn small primary" onClick={() => decide('approve')} disabled={busy}>
          Aprovar
        </button>
        <button type="button" className="btn small danger" onClick={() => decide('deny')} disabled={busy}>
          Negar
        </button>
      </div>
      {error && <span className="error-text">{error}</span>}
    </div>
  );
}

function FeedRow({ item, agents }: { item: FeedItem; agents: Record<string, AgentView> }) {
  const from = agents[item.from];
  const to = agents[item.to];
  const name = (id: string, agent: AgentView | undefined, fallback?: string) =>
    agent?.name ?? (id === 'user' ? 'Você' : (fallback ?? 'Agente'));
  return (
    <div className="feed-item" data-testid="feed-item">
      <div className="feed-head">
        <span className="swatch" style={{ background: agentColor(from) }} />
        <b>{name(item.from, from, item.fromName)}</b>
        <span className="to">para</span>
        <span className="swatch" style={{ background: agentColor(to) }} />
        <b>{name(item.to, to, item.toName)}</b>
        <span className="time">{formatTime(item.ts)}</span>
      </div>
      <p>{item.text}</p>
    </div>
  );
}

/** Pending approvals stay above the canvas, where they are seen. */
function Approvals({ agentIds }: { agentIds: string[] }) {
  const agents = useStore((state) => state.agents);
  const approvals = useStore((state) => state.approvals);
  const pending = approvals.filter((approval) => agentIds.includes(approval.agent_id));
  if (pending.length === 0) {
    return null;
  }
  return (
    <section className="card" aria-label="Aprovações pendentes">
      <div className="card-head">
        <h2>Aprovações pendentes</h2>
        <span className="note">{pending.length}</span>
      </div>
      {pending.map((approval) => (
        <ApprovalRow key={approval.id} approval={approval} agentName={agents[approval.agent_id]?.name ?? 'Agente'} />
      ))}
    </section>
  );
}

/** Conversation and spend, below the canvas; re-render together, at most once per batch. */
function SidePanel({ agentIds }: { agentIds: string[] }) {
  const agents = useStore((state) => state.agents);
  const feed = useStore((state) => state.feed);
  const [showAll, setShowAll] = useState(false);

  const inProject = useMemo(() => new Set(agentIds), [agentIds]);
  const conversation = useMemo(
    () => feed.filter((item) => inProject.has(item.from) || inProject.has(item.to)),
    [feed, inProject],
  );
  const shown = showAll ? conversation : conversation.slice(0, FEED_SHORT);

  const team = agentIds.map((id) => agents[id]).filter((agent): agent is AgentView => Boolean(agent));
  const spend = [...team].sort((a, b) => b.live.cost_usd - a.live.cost_usd);
  const total = spend.reduce((sum, agent) => sum + agent.live.cost_usd, 0);
  const max = spend[0]?.live.cost_usd ?? 0;

  return (
    <aside className="col-side" aria-label="Atividade entre agentes">
      <section className="card" aria-label="Conversa entre agentes">
        <div className="card-head">
          <h2>Conversa entre agentes</h2>
          <span className="note">ao vivo</span>
        </div>
        {shown.length === 0 && <p className="empty">Nenhuma mensagem trocada ainda.</p>}
        {shown.map((item) => (
          <FeedRow key={item.id} item={item} agents={agents} />
        ))}
        {conversation.length > FEED_SHORT && (
          <button
            type="button"
            className="btn small"
            style={{ alignSelf: 'flex-start' }}
            onClick={() => setShowAll((value) => !value)}
          >
            {showAll ? 'Mostrar menos' : 'Ver histórico completo'}
          </button>
        )}
      </section>

      <section className="card" aria-label="Gasto por agente" style={{ gap: 12 }}>
        <div className="card-head">
          <h2>Gasto por agente</h2>
          <b style={{ fontSize: 13 }}>{formatUsd(total)}</b>
        </div>
        {spend.map((agent) => (
          <div className="spend-row" key={agent.id}>
            <div className="line">
              <span>{agent.name}</span>
              <span>{formatUsd(agent.live.cost_usd)}</span>
            </div>
            <span className="bar tall">
              <span
                style={{
                  width: `${max > 0 ? Math.round((agent.live.cost_usd / max) * 100) : 0}%`,
                  background: agentColor(agent),
                }}
              />
            </span>
          </div>
        ))}
        <p className="empty" style={{ fontSize: 12 }}>
          Claude Code informa o custo direto; Codex e Antigravity CLI são estimados pelos tokens.
        </p>
      </section>
    </aside>
  );
}

function Alerts({ agentIds }: { agentIds: string[] }) {
  const alerts = useStore((state) => state.alerts);
  const dismissed = useStore((state) => state.dismissedAlerts);
  const names = useStore(useShallow((state) => agentIds.map((id) => state.agents[id]?.name ?? 'Agente')));
  const visible = alerts.filter((alert) => agentIds.includes(alert.agentId) && !dismissed.includes(alert.id));
  if (visible.length === 0) {
    return null;
  }
  return (
    <section aria-label="Alertas de segurança" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {visible.slice(0, 5).map((alert) => (
        <AlertRow key={alert.id} alert={alert} agentName={names[agentIds.indexOf(alert.agentId)] ?? 'Agente'} />
      ))}
    </section>
  );
}

function HeaderChips({ agentIds }: { agentIds: string[] }) {
  const active = useStore((state) => agentIds.filter((id) => isActive(state.agents[id]?.live.status)).length);
  const today = useStore((state) =>
    (state.usage?.agents ?? [])
      .filter((entry) => agentIds.includes(entry.agent_id))
      .reduce((sum, entry) => sum + (entry.windows.day?.cost_usd ?? 0), 0),
  );
  return (
    <div className="head-actions">
      <span className="chip">
        {agentIds.length} {agentIds.length === 1 ? 'agente' : 'agentes'} · {active} {active === 1 ? 'ativo' : 'ativos'}
      </span>
      <span className="chip">Hoje {formatUsd(today)}</span>
    </div>
  );
}

export function Overview({ projectId, onCreate }: { projectId?: string; onCreate: (kind: CreateKind) => void }) {
  const project = useStore((state) => state.projects.find((item) => item.id === projectId));
  // Orchestrator first, then the order of the daemon.
  const agentIds = useStore(
    useShallow((state) => {
      const ids = state.agentOrder.filter((id) => state.agents[id]?.project_id === projectId);
      return [
        ...ids.filter((id) => state.agents[id]?.is_orchestrator),
        ...ids.filter((id) => !state.agents[id]?.is_orchestrator),
      ];
    }),
  );
  const orchestratorName = useStore((state) => {
    const first = agentIds[0] ? state.agents[agentIds[0]] : undefined;
    return first?.is_orchestrator ? first.name : undefined;
  });

  if (!project) {
    return (
      <>
        <header className="page-head">
          <div>
            <div className="crumb">Regente</div>
            <h1>Nenhum projeto ainda</h1>
            <p className="lead">Crie um projeto para montar o time de agentes e enviar a primeira tarefa.</p>
          </div>
        </header>
        <div>
          <button type="button" className="btn primary" onClick={() => onCreate('project')}>
            Novo projeto
          </button>
        </div>
      </>
    );
  }

  return (
    <div className="ov-page">
      <header className="page-head">
        <div>
          <div className="crumb">{project.name} / Agentes</div>
          <h1>{orchestratorName ?? project.name}</h1>
        </div>
        <HeaderChips agentIds={agentIds} />
      </header>

      <div className="ov-top">
        <Alerts agentIds={agentIds} />
        <Approvals agentIds={agentIds} />
      </div>

      {/* The canvas comes first; the rest of the page is secondary. */}
      <div className="columns">
        <section className="col-main" aria-label="Agentes">
          <Canvas projectId={project.id} onCreateAgent={() => onCreate('agent')} />
        </section>
        <CurrentTask projectId={project.id} />
        <NewTaskBox project={project} agentIds={agentIds} />
        <SidePanel agentIds={agentIds} />
      </div>
    </div>
  );
}
