import { useEffect, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from 'react';
import { connectStream } from './api/stream';
import { MaestroChat } from './components/Chat';
import { AgentInspector, QuickInspector } from './components/Inspector';
import { Sidebar } from './components/Sidebar';
import { Topbar } from './components/Topbar';
import { parseRoute, routeHash, useHash } from './lib/route';
import { NewAgent } from './screens/NewAgent';
import { NewProject } from './screens/NewProject';
import { Overview } from './screens/Overview';
import { Profiles } from './screens/Profiles';
import { useStore, type PanelTab } from './store';
import { API_VERSION } from './api/version';
export type CreateKind = 'project' | 'agent' | 'settings';

const TABS: { id: PanelTab; label: string }[] = [
  { id: 'chat', label: 'Chat' },
  { id: 'activity', label: 'Atividade' },
  { id: 'files', label: 'Arquivos' },
];

const CHAT_MIN = 320;
const CHAT_MAX = 520;

interface Layout {
  chatWidth: number;
  sidebarExpanded: boolean;
}

const DEFAULT_LAYOUT: Layout = { chatWidth: 400, sidebarExpanded: false };
const layoutKey = (projectId = '') => `regente.layout.${projectId}`;
const clampChat = (width: number) => Math.min(CHAT_MAX, Math.max(CHAT_MIN, Math.round(width)));

function readLayout(projectId?: string): Layout {
  try {
    const saved = JSON.parse(localStorage.getItem(layoutKey(projectId)) ?? '{}') as Partial<Layout>;
    return {
      chatWidth: typeof saved.chatWidth === 'number' ? clampChat(saved.chatWidth) : DEFAULT_LAYOUT.chatWidth,
      sidebarExpanded: typeof saved.sidebarExpanded === 'boolean' ? saved.sidebarExpanded : DEFAULT_LAYOUT.sidebarExpanded,
    };
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function saveLayout(projectId: string | undefined, layout: Layout) {
  try {
    localStorage.setItem(layoutKey(projectId), JSON.stringify(layout));
  } catch {
    // Storage may be unavailable; the layout lasts for this session.
  }
}

/** Under 1100px the chat is an overlay, closed by default. */
const wide = () => window.matchMedia?.('(min-width: 1100px)').matches ?? true;

export function App() {
  const [creating, setCreating] = useState<CreateKind | null>(null);
  const route = parseRoute(useHash());
  const projects = useStore((s) => s.projects);
  const agentProject = useStore((s) => route.screen === 'agent' ? s.agents[route.agentId]?.project_id : undefined);
  const ready = useStore((s) => s.ready);
  const incompatibleVersion = useStore((s) => s.incompatibleVersion);
  const projectId = (route.screen === 'overview' ? route.projectId : agentProject) ?? projects[0]?.id;
  const [layout, setLayout] = useState(() => ({ projectId, ...readLayout(projectId) }));
  const [chatOpen, setChatOpen] = useState(wide);
  const selectedAgentId = useStore((s) => s.selectedAgentId);
  const panelTab = useStore((s) => s.panelTab);
  const panelAgentId = useStore((s) => s.panelAgentId);
  const routeAgentId = route.screen === 'agent' ? route.agentId : null;
  useEffect(() => connectStream(), []);

  // `#/agente/<id>` opens the Atividade tab and leaves the canvas in the center.
  useEffect(() => {
    const agent = routeAgentId ? useStore.getState().agents[routeAgentId] : undefined;
    if (!agent) return;
    useStore.getState().openPanel('activity', agent.id);
    setChatOpen(true);
    history.replaceState(null, '', routeHash({ screen: 'overview', projectId: agent.project_id }));
    window.dispatchEvent(new HashChangeEvent('hashchange'));
  }, [routeAgentId, ready]);

  // Opening the Atividade tab from the inspector also opens a closed panel.
  useEffect(() => {
    if (panelTab !== 'chat') setChatOpen(true);
  }, [panelTab, panelAgentId]);

  const talkTo = (agentId: string) => {
    useStore.getState().setChatRecipientId(agentId);
    useStore.getState().openPanel('chat');
    setChatOpen(true);
  };

  // Each project keeps its own widths.
  if (layout.projectId !== projectId) setLayout({ projectId, ...readLayout(projectId) });

  const update = (change: Partial<Layout>) => {
    const next = { ...layout, ...change };
    setLayout(next);
    saveLayout(projectId, { chatWidth: next.chatWidth, sidebarExpanded: next.sidebarExpanded });
  };

  const startResize = (event: PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = layout.chatWidth;
    let width = startWidth;
    const move = (moved: globalThis.PointerEvent) => {
      width = clampChat(startWidth + startX - moved.clientX);
      setLayout((current) => ({ ...current, chatWidth: width }));
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      update({ chatWidth: width });
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };

  const resizeByKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === 'ArrowLeft' ? 16 : event.key === 'ArrowRight' ? -16 : 0;
    if (step) {
      event.preventDefault();
      update({ chatWidth: clampChat(layout.chatWidth + step) });
    }
  };

  if (incompatibleVersion !== null) return <main className="main"><div className="banner" role="alert">Versão da API incompatível ou indisponível: interface {API_VERSION}, daemon {incompatibleVersion}. Atualize o Regente para usar versões compatíveis.</div></main>;
  const style = { '--chat-w': `${layout.chatWidth}px`, '--side-w': layout.sidebarExpanded ? '240px' : '60px' } as CSSProperties;
  return (
    <div className="shell" style={style} data-chat={chatOpen ? 'open' : 'closed'}>
      <Topbar projectId={projectId} chatOpen={chatOpen} onToggleChat={() => setChatOpen((open) => !open)} onSettings={() => setCreating('settings')} />
      <Sidebar
        route={route}
        activeProjectId={projectId}
        onCreate={setCreating}
        expanded={layout.sidebarExpanded}
        onToggle={() => update({ sidebarExpanded: !layout.sidebarExpanded })}
      />
      <main className="main">
        {!ready ? <p className="empty">Aguardando dados do daemon…</p> : route.screen === 'profiles' ? <Profiles /> : <Overview projectId={projectId} onCreate={setCreating} />}
      </main>
      {ready && selectedAgentId && route.screen !== 'profiles' && (
        <QuickInspector key={selectedAgentId} agentId={selectedAgentId} onTalk={() => talkTo(selectedAgentId)} />
      )}
      {/* Tabs are hidden with CSS, never unmounted: draft, scroll and recipient survive. */}
      <aside className="chat-panel" aria-label="Chat" data-testid="chat-panel">
        <div
          className="chat-resize"
          role="separator"
          aria-orientation="vertical"
          aria-label="Largura do chat"
          aria-valuemin={CHAT_MIN}
          aria-valuemax={CHAT_MAX}
          aria-valuenow={layout.chatWidth}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={resizeByKey}
        />
        <div className="panel-tabs" role="tablist" aria-label="Painel">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={panelTab === tab.id}
              disabled={tab.id !== 'chat' && !panelAgentId}
              onClick={() => useStore.getState().openPanel(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <div className="panel-body" role="tabpanel" aria-label="Chat" hidden={panelTab !== 'chat'}>
          {ready && projectId ? <MaestroChat key={projectId} projectId={projectId} /> : <p className="empty">Sem projeto selecionado.</p>}
        </div>
        {ready && panelAgentId && panelTab !== 'chat' && (
          <div className="panel-body" role="tabpanel" aria-label={panelTab === 'activity' ? 'Atividade' : 'Arquivos'}>
            <AgentInspector key={panelAgentId} agentId={panelAgentId} tab={panelTab} />
          </div>
        )}
      </aside>
      {creating === 'project' && <NewProject onClose={() => setCreating(null)} />}
      {creating === 'settings' && <NewProject project={projects.find((p) => p.id === projectId)} onClose={() => setCreating(null)} />}
      {creating === 'agent' && <NewAgent projectId={projectId} onClose={() => setCreating(null)} />}
    </div>
  );
}
