// Typed REST client for the daemon (`/api/v1`, same origin).

import type {
  Account,
  AccountModels,
  AccountStatus,
  AgentView,
  Approval,
  CreatedProject,
  LatestDiff,
  Message,
  NewAgentBody,
  NewProjectBody,
  NewTaskBody,
  PickedFolder,
  Project,
  ProjectPatch,
  Snapshot,
  Task,
  TimelinePage,
  UsageReport,
  WindowKey,
} from './types';

export const API_PREFIX = '/api/v1';

export type QueryValue = string | number | boolean | null | undefined;

/** Build an API path with an encoded query string; empty values are dropped. */
export function apiUrl(path: string, query?: Record<string, QueryValue>): string {
  const clean = path.startsWith('/') ? path : `/${path}`;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined && value !== null && value !== '') {
      params.set(key, String(value));
    }
  }
  const search = params.toString();
  return `${API_PREFIX}${clean}${search ? `?${search}` : ''}`;
}

/** Path of one agent resource, with the id safely encoded. */
export function agentPath(id: string, suffix = ''): string {
  return `/agents/${encodeURIComponent(id)}${suffix}`;
}

/** Path of one account resource, with the id safely encoded. */
export function accountPath(id: string, suffix = ''): string {
  return `/accounts/${encodeURIComponent(id)}${suffix}`;
}

/** WebSocket URL on the same host as the page (ws for http, wss for https). */
export function wsUrl(path: string, location: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${scheme}//${location.host}${apiUrl(path)}`;
}

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const response = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let parsed: unknown;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  if (!response.ok) {
    const message =
      parsed && typeof parsed === 'object' && 'error' in parsed
        ? String((parsed as { error: unknown }).error)
        : `Erro ${response.status}`;
    throw new ApiError(response.status, message);
  }
  return parsed as T;
}

const get = <T>(url: string) => request<T>('GET', url);
const post = <T>(url: string, body?: unknown) => request<T>('POST', url, body);
const patch = <T>(url: string, body: unknown) => request<T>('PATCH', url, body);

export const api = {
  health: () => get<{ ok: boolean; version: string; api_version: string }>(apiUrl('/health')),
  snapshot: () => get<Snapshot>(apiUrl('/snapshot')),

  listProjects: () => get<Project[]>(apiUrl('/projects')),
  createProject: (body: NewProjectBody) => post<CreatedProject>(apiUrl('/projects'), body),
  patchProject: (id: string, changes: ProjectPatch) => patch<Project>(apiUrl(`/projects/${encodeURIComponent(id)}`), changes),
  /** The daemon opens the system folder picker; `path` is null when cancelled. */
  pickFolder: () => post<PickedFolder>(apiUrl('/pick-folder')),

  listAgents: (project?: string) => get<AgentView[]>(apiUrl('/agents', { project })),
  createAgent: (body: NewAgentBody) => post<AgentView>(apiUrl('/agents'), body),
  getAgent: (id: string) => get<AgentView>(apiUrl(agentPath(id))),
  patchAgent: (id: string, changes: Partial<NewAgentBody>) => patch<AgentView>(apiUrl(agentPath(id)), changes),
  pauseAgent: (id: string, reason?: string) =>
    post<AgentView>(apiUrl(agentPath(id, '/pause')), reason ? { reason } : {}),
  resumeAgent: (id: string) => post<AgentView>(apiUrl(agentPath(id, '/resume'))),
  messageAgent: (id: string, text: string) => post<{ ok: boolean }>(apiUrl(agentPath(id, '/message')), { text }),
  timeline: (id: string, options: { before?: number | null; limit?: number } = {}) =>
    get<TimelinePage>(apiUrl(agentPath(id, '/timeline'), { before: options.before, limit: options.limit })),
  latestDiff: (id: string) => get<LatestDiff>(apiUrl(agentPath(id, '/diff/latest'))),

  listTasks: (project?: string) => get<Task[]>(apiUrl('/tasks', { project })),
  createTask: (body: NewTaskBody) => post<Task>(apiUrl('/tasks'), body),
  listMessages: () => get<Message[]>(apiUrl('/messages')),

  usage: (window?: WindowKey) => get<UsageReport>(apiUrl('/usage', { window })),
  listAccounts: () => get<Account[]>(apiUrl('/accounts')),
  patchAccount: (id: string, changes: Partial<Omit<Account, 'id' | 'provider'>>) =>
    patch<Account>(apiUrl(accountPath(id)), changes),
  accountModels: (id: string) => get<AccountModels>(apiUrl(accountPath(id, '/models'))),
  accountStatus: (id: string) => get<AccountStatus>(apiUrl(accountPath(id, '/status'))),

  listApprovals: (status?: string) => get<Approval[]>(apiUrl('/approvals', { status })),
  decideApproval: (id: string, decision: 'approve' | 'deny') =>
    post<Approval>(apiUrl(`/approvals/${encodeURIComponent(id)}`), { decision }),
};
