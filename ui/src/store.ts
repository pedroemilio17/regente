// Global UI state (Zustand). The live part is produced only by the reducer.

import { create } from 'zustand';
import { api } from './api/client';
import {
  applyApproval,
  applyEvents,
  emptyState,
  fromSnapshot,
  mergeAlerts,
  type Alert,
  type LiveState,
} from './api/reducer';
import type { Account, AgentEvent, Approval, Snapshot, Task, UsageReport } from './api/types';
import { API_VERSION } from './api/version';

const DISMISSED_KEY = 'regente.dismissedAlerts';

function loadDismissed(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? '[]');
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string') : [];
  } catch {
    return [];
  }
}

/** Stream health shown in the topbar; data is never cleared when it drops. */
export type Connection = 'online' | 'reconnecting' | 'offline' | 'degraded';

/** Tabs of the right-hand panel. Chat is the default and never changes by itself. */
export type PanelTab = 'chat' | 'activity' | 'files';

function seen(map: Record<string, string>, agentId: string, ts: string | null | undefined) {
  if (ts && (!map[agentId] || ts > map[agentId])) map[agentId] = ts;
}

export interface Store extends LiveState {
  /** True once the first snapshot arrived. */
  ready: boolean;
  connection: Connection;
  /** ISO time of the last event seen per agent id; kept across reconnects. */
  lastEventAt: Record<string, string>;
  /** Agent selected on the canvas. Never changes the chat recipient. */
  selectedAgentId: string | null;
  /** Task to highlight in the flow ("Ver no fluxo"). */
  focusTaskId: string | null;
  setSelectedAgentId: (id: string | null) => void;
  setFocusTaskId: (id: string | null) => void;
  panelTab: PanelTab;
  /** Agent shown in the Atividade and Arquivos tabs. */
  panelAgentId: string | null;
  /** Configuration section open in the Atividade tab. */
  configOpen: boolean;
  /** Chat recipient picked explicitly; null means the project's Maestro. */
  chatRecipientId: string | null;
  openPanel: (tab: PanelTab, agentId?: string, configOpen?: boolean) => void;
  setConfigOpen: (open: boolean) => void;
  setChatRecipientId: (id: string | null) => void;
  apiVersion: string | null;
  incompatibleVersion: string | null;
  setIncompatibleVersion: (version: string) => void;
  dismissedAlerts: string[];
  applySnapshot: (snapshot: Snapshot) => void;
  applyEvents: (events: AgentEvent[]) => void;
  applyApproval: (approval: Approval) => void;
  setUsage: (usage: UsageReport) => void;
  setTasks: (tasks: Task[]) => void;
  setConnection: (connection: Connection) => void;
  upsertAccount: (account: Account) => void;
  addAlerts: (alerts: Alert[]) => void;
  dismissAlert: (id: string) => void;
  /** Reload the full state over REST (after creating a project, for example). */
  refresh: () => Promise<void>;
}

export const useStore = create<Store>((set, get) => ({
  ...emptyState,
  ready: false,
  connection: 'reconnecting',
  lastEventAt: {},
  selectedAgentId: null,
  focusTaskId: null,
  setSelectedAgentId: (selectedAgentId) => set({ selectedAgentId }),
  setFocusTaskId: (focusTaskId) => set({ focusTaskId }),
  panelTab: 'chat',
  panelAgentId: null,
  configOpen: false,
  chatRecipientId: null,
  openPanel: (panelTab, agentId, configOpen = false) =>
    set((state) => ({ panelTab, panelAgentId: agentId ?? state.panelAgentId, configOpen: agentId ? configOpen : state.configOpen })),
  setConfigOpen: (configOpen) => set({ configOpen }),
  setChatRecipientId: (chatRecipientId) => set({ chatRecipientId }),
  apiVersion: null,
  incompatibleVersion: null,
  setIncompatibleVersion: (version) => set({ incompatibleVersion: version, ready: false }),
  dismissedAlerts: loadDismissed(),
  applySnapshot: (snapshot) => {
    if (snapshot.api_version !== API_VERSION) {
      set({ incompatibleVersion: typeof snapshot.api_version === 'string' ? snapshot.api_version : 'desconhecida', ready: false });
      return;
    }
    set((state) => {
      const lastEventAt = { ...state.lastEventAt };
      for (const agent of snapshot.agents) seen(lastEventAt, agent.id, agent.live?.last_ts);
      return { ...fromSnapshot(snapshot, state), lastEventAt, ready: true, apiVersion: snapshot.api_version, incompatibleVersion: null };
    });
  },
  applyEvents: (events) =>
    set((state) => {
      const lastEventAt = { ...state.lastEventAt };
      for (const event of events) seen(lastEventAt, event.agent_id, event.ts);
      return { ...applyEvents(state, events), lastEventAt };
    }),
  applyApproval: (approval) => set((state) => applyApproval(state, approval)),
  setUsage: (usage) => set({ usage }),
  setTasks: (tasks) => set({ tasks }),
  setConnection: (connection) => set({ connection }),
  upsertAccount: (account) =>
    set((state) => ({ accounts: state.accounts.map((item) => (item.id === account.id ? account : item)) })),
  addAlerts: (alerts) => set((state) => ({ alerts: mergeAlerts(state.alerts, alerts) })),
  dismissAlert: (id) => {
    const dismissedAlerts = [...get().dismissedAlerts, id].slice(-200);
    try {
      localStorage.setItem(DISMISSED_KEY, JSON.stringify(dismissedAlerts));
    } catch {
      // Storage may be unavailable; the alert stays dismissed for this session.
    }
    set({ dismissedAlerts });
  },
  refresh: async () => {
    get().applySnapshot(await api.snapshot());
  },
}));
