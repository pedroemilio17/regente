// Pure reducer: derives the live UI state from the snapshot plus event batches.
// `applyEventToLive` mirrors `Live::apply` in daemon/src/app.rs.

import type {
  Account,
  AgentEvent,
  AgentLive,
  AgentView,
  Approval,
  Message,
  Project,
  Snapshot,
  Task,
  UsageReport,
} from './types';

export const RECENT_CHANGES = 20;
export const FEED_LIMIT = 200;
export const ALERT_LIMIT = 50;

export interface FeedItem {
  id: string;
  ts: string;
  from: string;
  to: string;
  fromName?: string;
  toName?: string;
  text: string;
}

export interface Alert {
  id: string;
  ts: string;
  agentId: string;
  summary: string;
  target?: string;
  critical: boolean;
}

export interface LiveState {
  lastSeq: number;
  projects: Project[];
  agents: Record<string, AgentView>;
  /** Agent ids in the order the daemon lists them. */
  agentOrder: string[];
  tasks: Task[];
  accounts: Account[];
  usage: UsageReport | null;
  approvals: Approval[];
  /** Conversation between agents, newest first. */
  feed: FeedItem[];
  /** Security alerts, newest first. */
  alerts: Alert[];
}

export const emptyState: LiveState = {
  lastSeq: 0,
  projects: [],
  agents: {},
  agentOrder: [],
  tasks: [],
  accounts: [],
  usage: null,
  approvals: [],
  feed: [],
  alerts: [],
};

export function idleLive(): AgentLive {
  return {
    status: 'idle',
    doing_now: '',
    task_id: null,
    recent_changes: [],
    cost_usd: 0,
    tokens: 0,
    last_ts: null,
    pause_reason: null,
  };
}

function text(data: AgentEvent['data'], key: string): string | undefined {
  const value = data?.[key];
  return typeof value === 'string' ? value : undefined;
}

/** Apply one event to an agent's live state; returns a new object. */
export function applyEventToLive(previous: AgentLive, event: AgentEvent): AgentLive {
  const live: AgentLive = { ...previous, last_ts: event.ts };
  if (event.usage) {
    live.cost_usd += event.usage.cost_usd ?? 0;
    live.tokens += (event.usage.tokens_in ?? 0) + (event.usage.tokens_out ?? 0);
  }
  const paused = previous.status === 'paused';
  switch (event.kind) {
    case 'status': {
      const status = text(event.data, 'status') ?? 'idle';
      if (status === 'working' || status === 'thinking' || status === 'waiting' || status === 'idle' || status === 'paused') {
        live.status = status;
      }
      live.doing_now = event.summary;
      live.pause_reason = status === 'paused' ? (text(event.data, 'reason') ?? null) : null;
      if (event.task_id) {
        live.task_id = event.task_id;
      }
      break;
    }
    case 'thinking':
      if (!paused) {
        live.status = 'thinking';
        live.doing_now = event.summary;
      }
      break;
    case 'file_read':
    case 'file_edit':
    case 'command':
    case 'command_result':
      if (!paused) {
        live.status = 'working';
        live.doing_now = event.summary;
      }
      if (event.kind === 'file_edit' && event.target) {
        live.recent_changes = [
          {
            event_id: event.id,
            ts: event.ts,
            path: event.target,
            summary: event.summary,
            lines_added: event.lines_added ?? 0,
            lines_removed: event.lines_removed ?? 0,
          },
          ...previous.recent_changes,
        ].slice(0, RECENT_CHANGES);
      }
      break;
    case 'approval_requested':
      if (!paused) {
        live.status = 'waiting';
        live.doing_now = event.summary;
      }
      break;
    case 'task_assigned':
      live.task_id = event.task_id ?? null;
      break;
    case 'task_done':
      if (!paused) {
        live.status = 'idle';
        live.doing_now = event.summary;
      }
      if (previous.task_id === (event.task_id ?? null)) {
        live.task_id = null;
      }
      break;
    default:
      break;
  }
  return live;
}

export function feedFromMessage(message: Message): FeedItem {
  return {
    id: message.id,
    ts: message.created_at,
    from: message.from_agent,
    to: message.to_agent,
    text: message.text,
  };
}

/** Feed entry of an event, when the event is part of the conversation. */
export function feedFromEvent(event: AgentEvent): FeedItem | null {
  const fromUser = event.kind === 'message_received' && text(event.data, 'from') === 'user';
  if (event.kind !== 'message_sent' && !fromUser) {
    return null;
  }
  return {
    id: event.id,
    ts: event.ts,
    from: text(event.data, 'from') ?? event.agent_id,
    to: text(event.data, 'to') ?? '',
    fromName: text(event.data, 'from_name'),
    toName: text(event.data, 'to_name') ?? (event.kind === 'message_sent' ? event.target : undefined),
    text: text(event.data, 'text') ?? event.summary,
  };
}

export function alertFromEvent(event: AgentEvent): Alert | null {
  if (event.kind !== 'security_alert') {
    return null;
  }
  return {
    id: event.id,
    ts: event.ts,
    agentId: event.agent_id,
    summary: event.summary,
    target: event.target,
    critical: text(event.data, 'severity') === 'critical',
  };
}

/** Merge alerts keeping one per id, newest first. */
export function mergeAlerts(current: Alert[], incoming: Alert[]): Alert[] {
  if (incoming.length === 0) {
    return current;
  }
  const byId = new Map<string, Alert>();
  for (const alert of [...incoming, ...current]) {
    if (!byId.has(alert.id)) {
      byId.set(alert.id, alert);
    }
  }
  return [...byId.values()].sort((a, b) => b.ts.localeCompare(a.ts)).slice(0, ALERT_LIMIT);
}

/** Replace the state with a fresh snapshot. Alerts are kept: the snapshot has none. */
export function fromSnapshot(snapshot: Snapshot, previous: LiveState = emptyState): LiveState {
  const agents: Record<string, AgentView> = {};
  for (const agent of snapshot.agents) {
    agents[agent.id] = { ...agent, live: agent.live ?? idleLive() };
  }
  return {
    lastSeq: snapshot.last_seq,
    projects: snapshot.projects,
    agents,
    agentOrder: snapshot.agents.map((agent) => agent.id),
    tasks: snapshot.tasks,
    accounts: snapshot.accounts,
    usage: snapshot.usage,
    approvals: snapshot.approvals.filter((approval) => approval.status === 'pending'),
    feed: snapshot.messages
      .map(feedFromMessage)
      .sort((a, b) => b.ts.localeCompare(a.ts))
      .slice(0, FEED_LIMIT),
    alerts: previous.alerts,
  };
}

/**
 * Apply one batch of events. Events already covered by the snapshot
 * (seq <= lastSeq) are ignored, and agents without events keep their identity,
 * so only the cards that changed re-render.
 */
export function applyEvents(state: LiveState, events: AgentEvent[]): LiveState {
  let lastSeq = state.lastSeq;
  let agents = state.agents;
  let approvals = state.approvals;
  const feed: FeedItem[] = [];
  const alerts: Alert[] = [];
  let changed = false;

  for (const event of events) {
    if (event.seq <= lastSeq) {
      continue;
    }
    lastSeq = event.seq;
    changed = true;

    const agent = agents[event.agent_id];
    if (agent) {
      if (agents === state.agents) {
        agents = { ...state.agents };
      }
      const live = applyEventToLive(agent.live, event);
      agents[event.agent_id] = {
        ...agent,
        live,
        paused: live.status === 'paused',
        pause_reason: live.pause_reason,
      };
    }

    const item = feedFromEvent(event);
    if (item) {
      feed.unshift(item);
    }
    const alert = alertFromEvent(event);
    if (alert) {
      alerts.push(alert);
    }
    if (event.kind === 'approval_decided') {
      const id = text(event.data, 'approval_id');
      if (id && approvals.some((approval) => approval.id === id)) {
        approvals = approvals.filter((approval) => approval.id !== id);
      }
    }
  }

  if (!changed) {
    return state;
  }
  return {
    ...state,
    lastSeq,
    agents,
    approvals,
    feed: feed.length ? [...feed, ...state.feed].slice(0, FEED_LIMIT) : state.feed,
    alerts: mergeAlerts(state.alerts, alerts),
  };
}

/** Pending approvals are added (or replaced); decided ones leave the list. */
export function applyApproval(state: LiveState, approval: Approval): LiveState {
  const others = state.approvals.filter((item) => item.id !== approval.id);
  if (approval.status !== 'pending') {
    return others.length === state.approvals.length ? state : { ...state, approvals: others };
  }
  return { ...state, approvals: [...others, approval] };
}

/** True when the batch changes the task list, so the UI should reload it. */
export function touchesTasks(events: AgentEvent[]): boolean {
  return events.some((event) => event.kind === 'task_assigned' || event.kind === 'task_done');
}
