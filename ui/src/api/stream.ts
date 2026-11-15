// WebSocket client of `/api/v1/stream`: snapshot first, then batches.
// Reconnects with backoff and applies events at most every 250 ms.

import { api, ApiError, wsUrl } from './client';
import { API_VERSION } from './version';
import { alertFromEvent, touchesTasks, type Alert } from './reducer';
import type { AgentEvent, StreamMessage } from './types';
import { useStore } from '../store';

export const BATCH_MS = 250;

export interface Batcher<T> {
  push: (items: T[]) => void;
  cancel: () => void;
}

/**
 * Group items so `flush` runs at most once per `intervalMs`: the first push
 * after a quiet period flushes right away, later ones wait for the interval.
 */
export function createBatcher<T>(
  flush: (items: T[]) => void,
  intervalMs = BATCH_MS,
  now: () => number = () => performance.now(),
): Batcher<T> {
  let queue: T[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  let last = Number.NEGATIVE_INFINITY;

  const run = () => {
    timer = null;
    last = now();
    const items = queue;
    queue = [];
    if (items.length > 0) {
      flush(items);
    }
  };

  return {
    push(items) {
      if (items.length === 0) {
        return;
      }
      queue.push(...items);
      if (timer !== null) {
        return;
      }
      const wait = last + intervalMs - now();
      if (wait <= 0) {
        run();
      } else {
        timer = setTimeout(run, wait);
      }
    },
    cancel() {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      queue = [];
    },
  };
}

type EventListener = (events: AgentEvent[]) => void;
const eventListeners = new Set<EventListener>();

/**
 * Receive every applied batch (already grouped at 250 ms). Used by screens
 * that keep their own event list, like the agent timeline.
 */
export function subscribeEvents(listener: EventListener): () => void {
  eventListeners.add(listener);
  return () => {
    eventListeners.delete(listener);
  };
}

/** Delay before reconnect attempt `attempt` (0-based): 0.5 s doubling up to 10 s. */
export function backoffDelay(attempt: number): number {
  return Math.min(10_000, 500 * 2 ** Math.min(attempt, 10));
}

/**
 * The snapshot carries no security alerts, so recent ones are read from each
 * agent's timeline once per connection.
 */
async function backfillAlerts(agentIds: string[]): Promise<void> {
  const pages = await Promise.allSettled(agentIds.map((id) => api.timeline(id, { limit: 100 })));
  const alerts: Alert[] = [];
  for (const page of pages) {
    if (page.status !== 'fulfilled') {
      continue;
    }
    for (const event of page.value.items) {
      const alert = alertFromEvent(event);
      if (alert) {
        alerts.push(alert);
      }
    }
  }
  useStore.getState().addAlerts(alerts);
}

/** Open the stream and keep it open. Returns a function that stops it. */
export function connectStream(): () => void {
  let socket: WebSocket | null = null;
  let retry: ReturnType<typeof setTimeout> | null = null;
  let attempt = 0;
  let stopped = false;
  let backfilled = new Set<string>();
  let tasksTimer: ReturnType<typeof setTimeout> | null = null;
  let snapshotTimer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  let compatible = false;

  const checkHealth = async (currentGeneration: number) => {
    try {
      const health = await api.health();
      if (stopped || generation !== currentGeneration || compatible) return;
      if (!health || typeof health.api_version !== 'string') {
        useStore.getState().setIncompatibleVersion('desconhecida');
      } else if (health.api_version !== API_VERSION) {
        useStore.getState().setIncompatibleVersion(health.api_version);
      }
    } catch (error) {
      if (!stopped && generation === currentGeneration && !compatible && error instanceof ApiError && error.status === 404) {
        useStore.getState().setIncompatibleVersion('desconhecida');
      }
    }
  };

  const reloadTasks = () => {
    if (tasksTimer !== null) {
      return;
    }
    tasksTimer = setTimeout(() => {
      tasksTimer = null;
      api.listTasks().then(useStore.getState().setTasks, () => undefined);
    }, BATCH_MS);
  };

  const batcher = createBatcher<AgentEvent>((events) => {
    useStore.getState().applyEvents(events);
    eventListeners.forEach((listener) => listener(events));
    if (touchesTasks(events)) {
      reloadTasks();
    }
  });

  const handle = (message: StreamMessage) => {
    const store = useStore.getState();
    switch (message.type) {
      case 'snapshot': {
        store.applySnapshot(message);
        compatible = message.api_version === API_VERSION;
        if (snapshotTimer !== null) { clearTimeout(snapshotTimer); snapshotTimer = null; }
        if (!compatible) { batcher.cancel(); break; }
        store.setConnection('online');
        const fresh = message.agents.map((agent) => agent.id).filter((id) => !backfilled.has(id));
        if (fresh.length > 0) {
          fresh.forEach((id) => backfilled.add(id));
          void backfillAlerts(fresh);
        }
        break;
      }
      case 'events':
        if (!compatible) break;
        batcher.push(message.items);
        break;
      case 'usage':
        if (!compatible) break;
        store.setUsage(message.usage);
        break;
      case 'approval':
        if (!compatible) break;
        store.applyApproval(message.approval);
        break;
      default:
        break;
    }
  };

  const open = () => {
    if (stopped || socket !== null) {
      return;
    }
    const current = new WebSocket(wsUrl('/stream'));
    const currentGeneration = ++generation;
    compatible = false;
    snapshotTimer = setTimeout(() => {
      snapshotTimer = null;
      // Open but no snapshot yet: the daemon answers slowly or not at all.
      if (current.readyState === WebSocket.OPEN) useStore.getState().setConnection('degraded');
      void checkHealth(currentGeneration);
    }, 2000);
    socket = current;
    current.onopen = () => {
      attempt = 0;
      backfilled = new Set();
    };
    current.onmessage = (event) => {
      if (typeof event.data !== 'string') {
        return;
      }
      try {
        handle(JSON.parse(event.data) as StreamMessage);
      } catch {
        // A malformed frame is skipped; the next snapshot repairs the state.
      }
    };
    current.onclose = () => {
      if (socket !== current) return;
      socket = null;
      if (snapshotTimer !== null) { clearTimeout(snapshotTimer); snapshotTimer = null; }
      if (!compatible) void checkHealth(currentGeneration);
      compatible = false;
      batcher.cancel();
      // Data stays as it was; a few failed retries (or no network) mean offline.
      useStore.getState().setConnection(attempt >= 3 || !navigator.onLine ? 'offline' : 'reconnecting');
      if (!stopped) {
        retry = setTimeout(open, backoffDelay(attempt));
        attempt += 1;
      }
    };
    current.onerror = () => current.close();
  };

  const onOnline = () => {
    if (retry !== null) { clearTimeout(retry); retry = null; }
    if (socket === null) useStore.getState().setConnection('reconnecting');
    open();
  };
  window.addEventListener('online', onOnline);
  open();

  return () => {
    stopped = true;
    if (snapshotTimer !== null) clearTimeout(snapshotTimer);
    window.removeEventListener('online', onOnline);
    batcher.cancel();
    if (retry !== null) {
      clearTimeout(retry);
    }
    if (tasksTimer !== null) {
      clearTimeout(tasksTimer);
    }
    socket?.close();
  };
}
