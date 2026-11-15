import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { api } from '../api/client';
import { subscribeEvents } from '../api/stream';
import type { AgentEvent } from '../api/types';
import { formatTime } from '../lib/format';
import { cliInfo, isActive } from '../lib/labels';
import { useStore } from '../store';
import { AgentMarkdown } from './Markdown';
import { Timeline } from './Timeline';

type Item =
  | { type: 'bubble'; side: 'user' | 'agent'; id: string; ts: string; text: string }
  | { type: 'steps'; id: string; events: AgentEvent[] };

/** Text the user sent (shown on the right) or the agent said (on the left); everything else is a technical step. */
function bubbleOf(event: AgentEvent): { side: 'user' | 'agent'; text: string } | null {
  if (event.kind === 'message_received' && event.data?.from === 'user') {
    return { side: 'user', text: String(event.data.text ?? event.summary) };
  }
  // The CLI's own text and its final summary arrive as "thinking" and "task_done".
  if ((event.kind === 'thinking' && event.summary !== '' && event.summary !== 'Pensando') || event.kind === 'task_done') {
    const text = typeof event.data?.text === 'string' ? event.data.text : event.summary;
    return text ? { side: 'agent', text } : null;
  }
  return null;
}

/** `events` is newest first (as the timeline); the result is chronological, with technical events grouped. */
export function chatItems(events: AgentEvent[]): Item[] {
  const items: Item[] = [];
  for (const event of [...events].reverse()) {
    const bubble = bubbleOf(event);
    const last = items[items.length - 1];
    if (bubble) {
      items.push({ type: 'bubble', id: event.id, ts: event.ts, ...bubble });
    } else if (last?.type === 'steps') {
      last.events.unshift(event);
    } else {
      items.push({ type: 'steps', id: event.id, events: [event] });
    }
  }
  return items;
}

function errorText(cause: unknown, fallback: string): string {
  return cause instanceof Error && cause.message ? cause.message : fallback;
}

const PAGE = 50;
/** Keep the in-memory timeline bounded while events keep arriving. */
const MAX_EVENTS = 500;

/** Paginated timeline of one agent, kept fresh by the event stream. Mount with `key={agentId}`. */
export function useTimeline(agentId: string) {
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [nextBefore, setNextBefore] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    api.timeline(agentId, { limit: PAGE }).then(
      (page) => {
        if (cancelled) return;
        // Events that arrived while the page was loading stay on top.
        setEvents((live) => {
          const known = new Set(page.items.map((event) => event.id));
          return [...live.filter((event) => !known.has(event.id)), ...page.items];
        });
        setNextBefore(page.next_before);
        setLoading(false);
      },
      (cause) => {
        if (cancelled) return;
        setError(errorText(cause, 'Não foi possível carregar a conversa.'));
        setLoading(false);
      },
    );
    const unsubscribe = subscribeEvents((batch) => {
      const own = batch.filter((event) => event.agent_id === agentId);
      if (own.length === 0) return;
      setEvents((current) => {
        const known = new Set(current.map((event) => event.id));
        const fresh = own.filter((event) => !known.has(event.id)).reverse();
        return fresh.length ? [...fresh, ...current].slice(0, MAX_EVENTS) : current;
      });
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [agentId]);

  const loadMore = useCallback(() => {
    if (nextBefore === null) return;
    setLoading(true);
    api.timeline(agentId, { before: nextBefore, limit: PAGE }).then(
      (page) => {
        setEvents((current) => {
          const known = new Set(current.map((event) => event.id));
          return [...current, ...page.items.filter((event) => !known.has(event.id))];
        });
        setNextBefore(page.next_before);
        setLoading(false);
      },
      (cause) => {
        setError(errorText(cause, 'Não foi possível carregar mensagens anteriores.'));
        setLoading(false);
      },
    );
  }, [agentId, nextBefore]);

  return { events, hasMore: nextBefore !== null, loading, error, loadMore };
}

interface ChatProps {
  events: AgentEvent[];
  agentName: string;
  busy: boolean;
  hasMore: boolean;
  loading: boolean;
  onLoadMore: () => void;
}

/** Conversation view: bubbles for text, collapsed rows for tools, commands and status changes. */
export function Chat({ events, agentName, busy, hasMore, loading, onLoadMore }: ChatProps) {
  const items = useMemo(() => chatItems(events), [events]);
  const list = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const lastId = useRef<string | undefined>(undefined);
  const [unseen, setUnseen] = useState(0);

  // Follow new content only while the user is at the end; otherwise count what arrived.
  useLayoutEffect(() => {
    const node = list.current;
    const newest = items[items.length - 1]?.id;
    const grew = newest !== lastId.current && lastId.current !== undefined;
    lastId.current = newest;
    if (node && stick.current) {
      node.scrollTop = node.scrollHeight;
    } else if (grew) {
      setUnseen((count) => count + 1);
    }
  }, [items, busy]);

  const onScroll = () => {
    const node = list.current;
    if (!node) return;
    stick.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48;
    if (stick.current) setUnseen(0);
  };

  const toEnd = () => {
    const node = list.current;
    if (node) node.scrollTop = node.scrollHeight;
    stick.current = true;
    setUnseen(0);
  };

  return (
    <div className="chat-scroll">
      <div className="chat-list" ref={list} onScroll={onScroll} role="log" aria-live="polite" aria-label={`Conversa com ${agentName}`}>
        {hasMore && (
          <button type="button" className="btn small" onClick={onLoadMore} disabled={loading} style={{ alignSelf: 'center' }}>
            {loading ? 'Carregando…' : 'Carregar anteriores'}
          </button>
        )}
        {items.length === 0 && <p className="empty">{loading ? 'Carregando…' : `Nenhuma conversa ainda. Escreva para ${agentName}.`}</p>}
        {items.map((item) =>
          item.type === 'bubble' ? (
            <div key={item.id} className={`bubble ${item.side}`} data-side={item.side}>
              <span className="who">
                {item.side === 'user' ? 'Você' : agentName} · {formatTime(item.ts)}
              </span>
              {item.side === 'user' ? <p>{item.text}</p> : <AgentMarkdown text={item.text} />}
            </div>
          ) : (
            <div key={item.id} className="chat-steps">
              <details className="steps">
                <summary>
                  {item.events.length === 1 ? '1 ação do agente' : `${item.events.length} ações do agente`}
                </summary>
                <Timeline events={[...item.events].reverse()} hasMore={false} loading={false} onLoadMore={() => {}} />
              </details>
              {item.events
                .filter((event) => event.kind === 'task_assigned' && event.task_id)
                .map((event) => (
                  <button key={event.id} type="button" className="link-btn" onClick={() => useStore.getState().setFocusTaskId(event.task_id ?? null)}>
                    Ver no fluxo: {event.summary}
                  </button>
                ))}
            </div>
          ),
        )}
        {busy && (
          <p className="typing" role="status">
            {agentName} está trabalhando…
          </p>
        )}
      </div>
      {unseen > 0 && (
        <button type="button" className="unseen" onClick={toEnd}>
          {unseen === 1 ? '1 nova' : `${unseen} novas`}
        </button>
      )}
    </div>
  );
}

const draftKey = (projectId: string, agentId: string) => `regente.draft.${projectId}.${agentId}`;

function readDraft(projectId: string, agentId: string): string {
  try {
    return localStorage.getItem(draftKey(projectId, agentId)) ?? '';
  } catch {
    return '';
  }
}

function writeDraft(projectId: string, agentId: string, text: string) {
  try {
    if (text) localStorage.setItem(draftKey(projectId, agentId), text);
    else localStorage.removeItem(draftKey(projectId, agentId));
  } catch {
    // Storage may be unavailable; the draft lives only in memory.
  }
}

/** Conversation with one agent: timeline plus the message box. */
function Conversation({ agentId }: { agentId: string }) {
  const agent = useStore((state) => state.agents[agentId]);
  const timeline = useTimeline(agentId);
  const working = isActive(agent?.live.status);
  const name = agent?.name ?? 'agente';
  return (
    <>
      {timeline.error && <span className="error-text">{timeline.error}</span>}
      <Chat
        events={timeline.events}
        agentName={name}
        busy={working}
        hasMore={timeline.hasMore}
        loading={timeline.loading}
        onLoadMore={timeline.loadMore}
      />
    </>
  );
}

/**
 * Right-hand panel: always talks to the project's Maestro unless the user
 * picks someone else with the selector, by typing `@nome` or with "Falar com
 * este agente" in the inspector. Canvas selection never changes the recipient.
 */
export function MaestroChat({ projectId }: { projectId: string }) {
  const all = useStore((state) => state.agents);
  const order = useStore((state) => state.agentOrder);
  const agents = useMemo(
    () => order.flatMap((id) => (all[id]?.project_id === projectId ? [all[id]] : [])),
    [all, order, projectId],
  );
  const maestro = agents.find((agent) => agent.is_orchestrator);
  const picked = useStore((state) => state.chatRecipientId);
  const setPicked = useStore((state) => state.setChatRecipientId);
  const recipient = agents.find((agent) => agent.id === picked) ?? maestro;
  const recipientId = recipient?.id ?? '';
  const [text, setText] = useState(() => readDraft(projectId, recipientId));
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');

  // The draft belongs to project + recipient: switching recipient loads its own.
  const [shownFor, setShownFor] = useState(recipientId);
  if (shownFor !== recipientId) {
    setShownFor(recipientId);
    setText(readDraft(projectId, recipientId));
  }

  const choose = (id: string, rest?: string) => {
    if (rest !== undefined) {
      writeDraft(projectId, recipientId, '');
      writeDraft(projectId, id, rest);
    }
    setPicked(id === maestro?.id ? null : id);
  };

  const edit = (value: string) => {
    // "@Nome " at the start switches the recipient; the longest name wins.
    if (value.startsWith('@')) {
      const lower = value.toLowerCase();
      const match = [...agents]
        .sort((a, b) => b.name.length - a.name.length)
        .find((agent) => lower.startsWith(`@${agent.name.toLowerCase()} `));
      if (match) {
        const rest = value.slice(match.name.length + 2);
        choose(match.id, rest);
        if (match.id === recipientId) {
          setText(rest);
          writeDraft(projectId, recipientId, rest);
        }
        return;
      }
    }
    setText(value);
    writeDraft(projectId, recipientId, value);
  };

  const send = async () => {
    const clean = text.trim();
    if (!clean || sending || !recipient) return;
    setSending(true);
    setError('');
    try {
      await api.messageAgent(recipient.id, clean);
      setText('');
      writeDraft(projectId, recipient.id, '');
    } catch (cause) {
      setError(errorText(cause, 'Não foi possível enviar a mensagem.'));
    } finally {
      setSending(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void send();
  };

  return (
    <section className="maestro-chat" aria-label="Chat do Maestro">
      <header className="maestro-head">
        <h2>Maestro · {maestro ? cliInfo(maestro.cli).label : 'sem orquestrador'}</h2>
      </header>
      {recipient ? (
        <Conversation key={recipient.id} agentId={recipient.id} />
      ) : (
        <p className="empty">Este projeto ainda não tem agentes. Crie um orquestrador para conversar.</p>
      )}
      <form className="chat-composer" onSubmit={submit}>
        {recipient && (
          <label className="to-chip">
            <span>Para:</span>
            <select aria-label="Destinatário" value={recipient.id} onChange={(event) => choose(event.target.value)}>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.is_orchestrator ? `${agent.name} (Maestro)` : agent.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <textarea
          className="textarea"
          rows={3}
          value={text}
          disabled={!recipient}
          onChange={(event) => edit(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={recipient ? `Mensagem para ${recipient.name}… (@nome muda o destinatário)` : ''}
          aria-label={recipient ? `Mensagem para ${recipient.name}` : 'Mensagem'}
        />
        <button type="submit" className="btn primary" disabled={sending || !text.trim() || !recipient}>
          {isActive(recipient?.live.status) ? 'Enviar à fila' : 'Enviar'}
        </button>
        {error && (
          <span className="error-text" role="alert">
            {error}
          </span>
        )}
      </form>
    </section>
  );
}
