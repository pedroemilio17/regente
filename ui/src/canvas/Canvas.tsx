// Agent canvas: Structure (configured topology) and Execution (what happened).
// Positions pinned by dragging, the viewport and the Graph/List choice live in
// localStorage per project. Everything is remounted per project, so events and
// late timeline responses of the previous project never reach the new one.

import { useEffect, useMemo, useRef, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import {
  Background,
  BackgroundVariant,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type NodeChange,
  type Viewport,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import './canvas.css';
import { api } from '../api/client';
import { subscribeEvents } from '../api/stream';
import type { AgentEvent, AgentView } from '../api/types';
import { useAnimationGate } from '../hooks/useAnimationGate';
import { useNow } from '../hooks/useNow';
import { formatTime } from '../lib/format';
import {
  agentStatus,
  executionEdges,
  inRun,
  parentMap,
  runRoots,
  structureEdges,
  TASK_STATUS_LABEL,
  taskStatus,
  type EdgeKind,
  type FlowEdge,
} from '../lib/projection';
import { navigate } from '../lib/route';
import { useStore } from '../store';
import { AgentList } from './AgentList';
import { AgentNode, type AgentFlowNode } from './AgentNode';
import { placeNodes, topologyKey, treeLayout, type Point } from './layout';

export type CanvasMode = 'structure' | 'execution';
export type CanvasView = 'graph' | 'list';

const NODE_TYPES = { orchestrator: AgentNode, agent: AgentNode };
const EVENT_LIMIT = 2000;
const FRESH_MS = 700;
/** Fit leaves room for the toolbar (top) and the zoom buttons (bottom). */
const FIT = { maxZoom: 1, padding: { top: '72px', bottom: '64px', left: '24px', right: '24px' } } as const;

const EDGE_LABEL: Record<EdgeKind, string> = {
  allowed: 'Pode delegar',
  delegated: 'Delegou',
  depends_on: 'Depende de',
  blocked_by: 'Bloqueada por',
  failed: 'Falhou',
};

interface Saved {
  /** Pinned in Structure; Execution shows the same arrangement, read-only. */
  pinned: Record<string, Point>;
  viewport: Partial<Record<CanvasMode, Viewport>>;
  view: CanvasView;
}

const storageKey = (projectId: string) => `regente.canvas.${projectId}`;

function load(projectId: string): Saved {
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey(projectId)) ?? '{}') as Partial<Saved>;
    return { pinned: parsed.pinned ?? {}, viewport: parsed.viewport ?? {}, view: parsed.view === 'list' ? 'list' : 'graph' };
  } catch {
    return { pinned: {}, viewport: {}, view: 'graph' };
  }
}

function save(projectId: string, change: (saved: Saved) => Saved): void {
  try {
    localStorage.setItem(storageKey(projectId), JSON.stringify(change(load(projectId))));
  } catch {
    // Storage may be unavailable; the layout just is not remembered.
  }
}

function mergeEvents(current: AgentEvent[], incoming: AgentEvent[]): AgentEvent[] {
  if (incoming.length === 0) {
    return current;
  }
  const bySeq = new Map(current.map((event) => [event.seq, event]));
  incoming.forEach((event) => bySeq.set(event.seq, event));
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq).slice(-EVENT_LIMIT);
}

function typing(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  return Boolean(element?.closest?.('input, textarea, select, [contenteditable="true"]'));
}

interface CanvasProps {
  projectId: string;
  onCreateAgent: () => void;
}

export function Canvas(props: CanvasProps) {
  return <ProjectCanvas key={props.projectId} {...props} />;
}

function ProjectCanvas({ projectId, onCreateAgent }: CanvasProps) {
  const [mode, setMode] = useState<CanvasMode>('structure');
  const [view, setView] = useState<CanvasView>(() => load(projectId).view);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [runChoice, setRunChoice] = useState<string | null>(null);
  const focusTaskId = useStore((state) => state.focusTaskId);
  const tasks = useStore((state) => state.tasks);
  const agents = useStore(
    useShallow((state) =>
      state.agentOrder
        .map((id) => state.agents[id])
        .filter((agent): agent is AgentView => agent?.project_id === projectId)
        .sort((a, b) => Number(b.is_orchestrator) - Number(a.is_orchestrator)),
    ),
  );
  const idsKey = agents.map((agent) => agent.id).join('|');
  const gate = useAnimationGate<HTMLDivElement>();

  // Execution edges show one run: the newest active root task, or the chosen one.
  const runs = useMemo(() => runRoots(tasks, projectId), [tasks, projectId]);
  const chosenRun = tasks.find((task) => task.id === runChoice && task.project_id === projectId);
  const runId = chosenRun?.id ?? runs[0]?.id ?? null;
  const runOptions = chosenRun && !runs.includes(chosenRun) ? [chosenRun, ...runs] : runs;

  // "Ver no fluxo" shows the delegation, and its run, in Execution mode.
  const [shownFocus, setShownFocus] = useState<string | null>(null);
  if (focusTaskId && focusTaskId !== shownFocus) {
    setShownFocus(focusTaskId);
    setMode('execution');
    setView('graph');
    const focusRoot = tasks.find((task) => task.id === focusTaskId)?.root_task_id;
    if (focusRoot) setRunChoice(focusRoot);
  }

  const chooseView = (next: CanvasView) => {
    setView(next);
    save(projectId, (saved) => ({ ...saved, view: next }));
  };

  // Recent history from each timeline, then every live batch.
  useEffect(() => {
    const ids = new Set(idsKey.split('|').filter(Boolean));
    let alive = true;
    const add = (incoming: AgentEvent[]) => {
      const mine = incoming.filter((event) => ids.has(event.agent_id));
      if (alive && mine.length > 0) {
        setEvents((current) => mergeEvents(current, mine));
      }
    };
    const stop = subscribeEvents(add);
    void Promise.allSettled([...ids].map((id) => api.timeline(id, { limit: 100 }))).then((pages) =>
      add(pages.flatMap((page) => (page.status === 'fulfilled' ? page.value.items : []))),
    );
    return () => {
      alive = false;
      stop();
    };
  }, [idsKey]);

  return (
    <div className="cv-wrap" ref={gate} data-testid="canvas">
      <div className="cv-toolbar">
        <div className="cv-modes" role="group" aria-label="Visualização">
          <button type="button" className="btn small" aria-pressed={view === 'graph'} onClick={() => chooseView('graph')}>
            Grafo
          </button>
          <button type="button" className="btn small" aria-pressed={view === 'list'} onClick={() => chooseView('list')}>
            Lista
          </button>
        </div>
        {view === 'graph' && (
          <div className="cv-modes" role="group" aria-label="Modo do canvas">
            <button type="button" className="btn small" aria-pressed={mode === 'structure'} onClick={() => setMode('structure')}>
              Estrutura
            </button>
            <button type="button" className="btn small" aria-pressed={mode === 'execution'} onClick={() => setMode('execution')}>
              Execução
            </button>
          </div>
        )}
        {view === 'graph' && mode === 'execution' && runOptions.length > 1 && (
          <select className="select cv-run" aria-label="Execução mostrada" value={runId ?? ''} onChange={(event) => setRunChoice(event.target.value)}>
            {runOptions.map((run) => (
              <option key={run.id} value={run.id}>
                {run.title}
              </option>
            ))}
          </select>
        )}
        {(view === 'list' || mode === 'structure') && (
          <button type="button" className="btn small" onClick={onCreateAgent}>
            Novo agente
          </button>
        )}
      </div>
      {agents.length === 0 ? (
        <p className="empty cv-empty">Este projeto ainda não tem agentes.</p>
      ) : view === 'list' ? (
        <AgentList agents={agents} events={events} />
      ) : (
        <ReactFlowProvider key={mode}>
          <Flow projectId={projectId} agents={agents} mode={mode} runId={runId} events={events} />
        </ReactFlowProvider>
      )}
    </div>
  );
}

interface FlowProps {
  projectId: string;
  agents: AgentView[];
  mode: CanvasMode;
  runId: string | null;
  events: AgentEvent[];
}

function Flow({ projectId, agents, mode, runId, events }: FlowProps) {
  const flow = useReactFlow();
  const now = useNow();
  const tasks = useStore((state) => state.tasks);
  const connected = useStore((state) => state.connection === 'online' || state.connection === 'degraded');
  const selectedAgentId = useStore((state) => state.selectedAgentId);
  const setSelectedAgentId = useStore((state) => state.setSelectedAgentId);
  const focusTaskId = useStore((state) => state.focusTaskId);
  const setFocusTaskId = useStore((state) => state.setFocusTaskId);

  const [initial] = useState(() => load(projectId));
  const [pinned, setPinned] = useState(initial.pinned);
  const [drag, setDrag] = useState<{ id: string; position: Point } | null>(null);
  const [measured, setMeasured] = useState<Record<string, { width: number; height: number }>>({});
  const [clickedEdge, setClickedEdge] = useState<string | null>(null);
  const [fresh, setFresh] = useState<string[]>([]);

  const parents = parentMap(agents);
  const items = agents.map((agent) => ({ id: agent.id, parentId: parents.get(agent.id) ?? null }));
  const topology = topologyKey(items);
  // Recomputed only when ids or parents change, never on status updates.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const layout = useMemo(() => treeLayout(items), [topology]);
  const positions = placeNodes(items, layout, pinned);

  const flowEdges: FlowEdge[] = useMemo(
    () => {
      if (mode === 'structure') return structureEdges(agents);
      const run = runId ? inRun(tasks, events, runId) : { tasks, events };
      return executionEdges(agents, run.tasks, run.events);
    },
    [mode, agents, tasks, events, runId],
  );

  // Highlight edges that appear after the first render, for ~700 ms.
  const known = useRef<Set<string> | null>(null);
  const edgeKey = flowEdges.map((edge) => edge.id).join('|');
  useEffect(() => {
    const ids = edgeKey.split('|').filter(Boolean);
    const before = known.current;
    known.current = new Set(ids);
    const added = before ? ids.filter((id) => !before.has(id)) : [];
    if (added.length === 0) {
      return;
    }
    setFresh((current) => [...current, ...added]);
    const timer = setTimeout(() => setFresh((current) => current.filter((id) => !added.includes(id))), FRESH_MS);
    return () => clearTimeout(timer);
  }, [edgeKey]);

  const selectedEdgeId = clickedEdge ?? (focusTaskId ? `delegated:${focusTaskId}` : null);
  const selectedEdge = flowEdges.find((edge) => edge.id === selectedEdgeId);

  const neighbors = new Set<string>();
  if (selectedAgentId) {
    neighbors.add(selectedAgentId);
    for (const edge of flowEdges) {
      if (edge.source === selectedAgentId) neighbors.add(edge.target);
      if (edge.target === selectedAgentId) neighbors.add(edge.source);
    }
  }
  const selectionHere = selectedAgentId !== null && agents.some((agent) => agent.id === selectedAgentId);

  const nodes: AgentFlowNode[] = agents.map((agent) => ({
    id: agent.id,
    type: agent.is_orchestrator ? 'orchestrator' : 'agent',
    position: drag?.id === agent.id ? drag.position : (positions[agent.id] as Point),
    measured: measured[agent.id],
    selected: agent.id === selectedAgentId,
    draggable: mode === 'structure',
    connectable: false,
    ariaLabel: agent.name,
    data: {
      agent,
      projection: agentStatus(agent, tasks, events, now, connected),
      now,
      dim: selectionHere && !neighbors.has(agent.id),
    },
  }));

  const edges: Edge[] = flowEdges.map((edge) => {
    const classes = ['cv-edge', `cv-edge-${edge.kind}`];
    if (selectionHere && (edge.source === selectedAgentId || edge.target === selectedAgentId)) classes.push('is-hl');
    if (fresh.includes(edge.id)) classes.push('is-new');
    if (edge.id === selectedEdgeId) classes.push('is-selected');
    return {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      className: classes.join(' '),
      selectable: mode === 'execution',
      focusable: mode === 'execution',
      ariaLabel: EDGE_LABEL[edge.kind],
      label: edge.kind === 'failed' || edge.kind === 'blocked_by' ? EDGE_LABEL[edge.kind] : undefined,
      markerEnd: edge.kind === 'allowed' ? undefined : { type: MarkerType.ArrowClosed },
    };
  });

  const onNodesChange = (changes: NodeChange<AgentFlowNode>[]) => {
    for (const change of changes) {
      if (change.type === 'dimensions' && change.dimensions) {
        const size = change.dimensions;
        setMeasured((current) => ({ ...current, [change.id]: size }));
      } else if (change.type === 'position' && change.position && change.dragging) {
        setDrag({ id: change.id, position: change.position });
      }
    }
  };

  const pin = (id: string, position: Point) => {
    setDrag(null);
    setPinned((current) => ({ ...current, [id]: position }));
    save(projectId, (saved) => ({ ...saved, pinned: { ...saved.pinned, [id]: position } }));
  };

  const clear = () => {
    setSelectedAgentId(null);
    setClickedEdge(null);
    if (focusTaskId) setFocusTaskId(null);
  };
  const clearRef = useRef(clear);
  useEffect(() => {
    clearRef.current = clear;
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey || event.metaKey || event.altKey || typing(event.target)) {
        return;
      }
      if (event.key === 'Escape') {
        clearRef.current();
      } else if (event.key === 'f' || event.key === 'F') {
        void flow.fitView({ ...FIT, duration: 200 });
      } else if (event.key === 'Enter') {
        const id = (event.target as HTMLElement | null)?.closest?.('.react-flow__node')?.getAttribute('data-id');
        if (id) navigate({ screen: 'agent', agentId: id });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [flow]);

  const name = (id: string) => agents.find((agent) => agent.id === id)?.name ?? 'Agente';
  const edgeTask = selectedEdge?.taskId ? tasks.find((task) => task.id === selectedEdge.taskId) : undefined;
  const viewport = initial.viewport[mode];

  return (
    <>
      <ReactFlow<AgentFlowNode, Edge>
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        onNodesChange={onNodesChange}
        onNodeDragStop={(_, node) => pin(node.id, node.position)}
        onNodeClick={(_, node) => {
          setClickedEdge(null);
          setSelectedAgentId(node.id);
        }}
        onNodeDoubleClick={(_, node) => navigate({ screen: 'agent', agentId: node.id })}
        onEdgeClick={(_, edge) => {
          if (mode === 'execution') setClickedEdge(edge.id);
        }}
        // Enter or Space on a focused edge selects it: same details as a click.
        onEdgesChange={(changes) => {
          const picked = changes.find((change) => change.type === 'select' && change.selected);
          if (picked && 'id' in picked && mode === 'execution') setClickedEdge(picked.id);
        }}
        onPaneClick={clear}
        onMoveEnd={(_, next) => save(projectId, (saved) => ({ ...saved, viewport: { ...saved.viewport, [mode]: next } }))}
        defaultViewport={viewport}
        fitView={!viewport}
        fitViewOptions={FIT}
        nodesConnectable={false}
        elementsSelectable
        deleteKeyCode={null}
        selectionKeyCode={null}
        multiSelectionKeyCode={null}
        minZoom={0.2}
        maxZoom={2}
        colorMode="dark"
      >
        <Background variant={BackgroundVariant.Dots} gap={24} size={1} />
      </ReactFlow>
      <div className="cv-controls" role="group" aria-label="Zoom">
        <button type="button" className="btn small" aria-label="Aproximar" onClick={() => void flow.zoomIn()}>
          +
        </button>
        <button type="button" className="btn small" aria-label="Afastar" onClick={() => void flow.zoomOut()}>
          −
        </button>
        <button type="button" className="btn small" aria-label="Ajustar à tela (F)" onClick={() => void flow.fitView({ ...FIT, duration: 200 })}>
          Ajustar
        </button>
      </div>
      {mode === 'execution' && selectedEdge && (
        <div className="cv-popover" role="dialog" aria-label="Detalhes da conexão" data-testid="edge-popover">
          <b>{EDGE_LABEL[selectedEdge.kind]}</b>
          <dl>
            <dt>Origem</dt>
            <dd>{name(selectedEdge.source)}</dd>
            <dt>Destino</dt>
            <dd>{name(selectedEdge.target)}</dd>
            {selectedEdge.ts && (
              <>
                <dt>Hora</dt>
                <dd>{formatTime(selectedEdge.ts)}</dd>
              </>
            )}
            <dt>Tarefa</dt>
            <dd>{edgeTask?.title ?? 'Tarefa não carregada'}</dd>
            {edgeTask && (
              <>
                <dt>Resultado</dt>
                <dd>{edgeTask.summary || TASK_STATUS_LABEL[taskStatus(edgeTask, events)]}</dd>
              </>
            )}
          </dl>
          <button type="button" className="btn small" onClick={clear}>
            Fechar
          </button>
        </div>
      )}
    </>
  );
}
