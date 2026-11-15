import { memo, useEffect, useRef, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { cliInfo } from '../lib/labels';
import { formatUsd } from '../lib/format';
import type { CreateKind } from '../App';
import { navigate, type Route } from '../lib/route';
import { useStore } from '../store';
import { BoltIcon, Logo, PluginIcon, PlusIcon, ProjectIcon, SearchIcon, SlidersIcon } from './Icons';
import { Pet } from './Pet';
import { StatusDot } from './Status';
import { UsageBars } from './UsageBars';

function CreateMenu({ onCreate }: { onCreate: (kind: CreateKind) => void }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onPointer = (event: PointerEvent) => {
      if (wrap.current && !wrap.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };
    document.addEventListener('pointerdown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const go = (kind: CreateKind) => {
    setOpen(false);
    // The menu unmounts: the dialog gives focus back to the Criar button.
    toggle.current?.focus();
    onCreate(kind);
  };

  return (
    <div className="create-wrap" ref={wrap}>
      <button
        ref={toggle}
        type="button"
        className="nav-item"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Criar"
        title="Criar"
        onClick={() => setOpen((value) => !value)}
      >
        <PlusIcon />
        <span className="label">Criar</span>
      </button>
      {open && (
        <div className="create-menu" role="menu">
          <button type="button" role="menuitem" onClick={() => go('agent')}>
            <span className="letter">A</span>
            <span>
              <span className="title">Novo agente</span>
              <span className="hint">Motor e função no projeto atual</span>
            </span>
          </button>
          <button type="button" role="menuitem" onClick={() => go('project')}>
            <span className="letter">P</span>
            <span>
              <span className="title">Novo projeto</span>
              <span className="hint">Nome, pasta e descrição</span>
            </span>
          </button>
        </div>
      )}
    </div>
  );
}

const AgentRow = memo(function AgentRow({ agentId, current }: { agentId: string; current: boolean }) {
  const agent = useStore((state) => state.agents[agentId]);
  if (!agent) {
    return null;
  }
  return (
    <a
      className="nav-item nav-agent"
      href={`#/agente/${encodeURIComponent(agent.id)}`}
      aria-current={current ? 'page' : undefined}
      data-testid="sidebar-agent"
    >
      <Pet pet={agent.pet} agent={agent} status={agent.live.status} size={agent.is_orchestrator ? 20 : 18} />
      <span className="name" style={{ fontWeight: agent.is_orchestrator || current ? 600 : 400 }}>
        {agent.name}
      </span>
      <StatusDot status={agent.live.status} title />
      <span className="cost">{formatUsd(agent.live.cost_usd)}</span>
    </a>
  );
});

function ProjectTree({ projectId, currentAgentId }: { projectId: string; currentAgentId?: string }) {
  // Two flat id lists: shallow comparison keeps them stable between batches.
  const orchestrators = useStore(
    useShallow((state) =>
      state.agentOrder.filter((id) => {
        const agent = state.agents[id];
        return agent?.project_id === projectId && agent.is_orchestrator;
      }),
    ),
  );
  const others = useStore(
    useShallow((state) =>
      state.agentOrder.filter((id) => {
        const agent = state.agents[id];
        return agent?.project_id === projectId && !agent.is_orchestrator;
      }),
    ),
  );
  if (orchestrators.length === 0 && others.length === 0) {
    return <div className="nav-empty">Nenhum agente ainda</div>;
  }
  return (
    <>
      {orchestrators.map((id) => (
        <AgentRow key={id} agentId={id} current={id === currentAgentId} />
      ))}
      {others.length > 0 && (
        <div className="nav-team">
          {others.map((id) => (
            <AgentRow key={id} agentId={id} current={id === currentAgentId} />
          ))}
        </div>
      )}
    </>
  );
}

const UsageCard = memo(function UsageCard() {
  const usage = useStore((state) => state.usage);
  const accounts = usage?.accounts ?? [];
  return (
    <section className="usage-card" aria-label="Uso dos planos">
      <div className="usage-head">
        <span className="muted">Uso dos planos</span>
      </div>
      {accounts.map((account) => {
        const cli = cliInfo(account.provider);
        return (
          <div key={account.account_id} className={`usage-account tone-${cli.key}`} data-testid="usage-account">
            <div className="usage-title">
              <span className="swatch" />
              <span className="name">{cli.provider}</span>
            </div>
            <UsageBars windows={account.windows} />
          </div>
        );
      })}
      {accounts.length === 0 && <span className="empty">Sem contas conectadas.</span>}
    </section>
  );
});

interface SidebarProps {
  route: Route;
  activeProjectId?: string;
  onCreate: (kind: CreateKind) => void;
  /** 240px with labels; collapsed it is a 60px column of icons with tooltips. */
  expanded: boolean;
  onToggle: () => void;
}

export function Sidebar({ route, activeProjectId, onCreate, expanded, onToggle }: SidebarProps) {
  const projects = useStore((state) => state.projects);
  const accountCount = useStore((state) => state.accounts.length);
  const inspected = useStore((state) => (state.panelTab === 'chat' ? null : state.panelAgentId));
  const currentAgentId = route.screen === 'agent' ? route.agentId : inspected ?? undefined;

  return (
    <nav className={`sidebar${expanded ? '' : ' collapsed'}`} aria-label="Navegação principal">
      <button
        type="button"
        className="nav-item brand"
        aria-expanded={expanded}
        aria-label={expanded ? 'Recolher menu' : 'Expandir menu'}
        title={expanded ? 'Recolher menu' : 'Expandir menu'}
        onClick={onToggle}
      >
        <Logo />
        <span className="label">Regente</span>
      </button>
      <CreateMenu onCreate={onCreate} />
      <button type="button" className="nav-item" disabled aria-label="Procurar" title="Procurar: disponível em uma próxima versão">
        <SearchIcon />
        <span className="label">Procurar</span>
      </button>
      <button type="button" className="nav-item" disabled aria-label="Skills" title="Skills: disponível em uma próxima versão">
        <BoltIcon />
        <span className="label">Skills</span>
        <span className="soon label">em breve</span>
      </button>
      <button type="button" className="nav-item" disabled aria-label="Plugins" title="Plugins: disponível em uma próxima versão">
        <PluginIcon />
        <span className="label">Plugins</span>
        <span className="soon label">em breve</span>
      </button>

      <div className="nav-heading label">Projetos</div>
      {expanded && projects.length === 0 && <div className="nav-empty">Nenhum projeto. Use Criar para começar.</div>}
      {projects.map((project) => {
        const active = project.id === activeProjectId;
        return (
          <div key={project.id}>
            <a
              className={`nav-item nav-project${active ? '' : ' dim'}`}
              href={`#/projeto/${encodeURIComponent(project.id)}`}
              aria-current={active && route.screen === 'overview' ? 'page' : undefined}
              aria-label={project.name}
              title={project.name}
              data-testid="sidebar-project"
            >
              <ProjectIcon />
              <span className="label">{project.name}</span>
            </a>
            {expanded && active && <ProjectTree projectId={project.id} currentAgentId={currentAgentId} />}
          </div>
        );
      })}

      <div className="spacer" />
      {expanded && <UsageCard />}
      <button
        type="button"
        className="profile-button"
        aria-label="Contas e perfis das IAs"
        title="Contas e perfis"
        aria-current={route.screen === 'profiles' ? 'page' : undefined}
        onClick={() => navigate({ screen: 'profiles' })}
      >
        <span className="avatar">V</span>
        <span className="who label">
          <span>Contas e perfis</span>
          <small>
            {accountCount} {accountCount === 1 ? 'conta de IA conectada' : 'contas de IA conectadas'}
          </small>
        </span>
        <span className="label">
          <SlidersIcon />
        </span>
      </button>
    </nav>
  );
}
