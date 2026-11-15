// Labels and colors for statuses, CLIs, providers and event kinds (pt-BR).

import type { AgentEvent, AgentView, EventKind } from '../api/types';

export interface StatusStyle {
  label: string;
  /** Lowercase label, as used inside sentences. */
  lower: string;
  /** CSS class suffix: st-working, st-thinking… */
  key: 'working' | 'thinking' | 'waiting' | 'idle' | 'paused';
  /** True when the dot pulses. */
  live: boolean;
}

const STATUS: Record<string, StatusStyle> = {
  working: { label: 'Trabalhando', lower: 'trabalhando', key: 'working', live: true },
  thinking: { label: 'Pensando', lower: 'pensando', key: 'thinking', live: true },
  waiting: { label: 'Aguardando', lower: 'aguardando', key: 'waiting', live: false },
  idle: { label: 'Disponível', lower: 'disponível', key: 'idle', live: false },
  paused: { label: 'Pausado', lower: 'pausado', key: 'paused', live: false },
};

export function statusStyle(status: string | undefined): StatusStyle {
  return STATUS[status ?? 'idle'] ?? (STATUS.idle as StatusStyle);
}

export function isActive(status: string | undefined): boolean {
  return status === 'working' || status === 'thinking';
}

export interface CliInfo {
  id: string;
  label: string;
  /** Account name shown on the usage card. */
  provider: string;
  /** CSS class suffix: cli-claude… */
  key: 'claude' | 'codex' | 'agy' | 'fake';
  runs: string;
}

export const CLIS: CliInfo[] = [
  { id: 'claude', label: 'Claude Code', provider: 'Claude', key: 'claude', runs: 'claude -p --output-format stream-json' },
  { id: 'codex', label: 'Codex', provider: 'ChatGPT', key: 'codex', runs: 'codex exec --json' },
  { id: 'agy', label: 'Antigravity CLI', provider: 'Antigravity', key: 'agy', runs: 'agy -p --output-format stream-json' },
  { id: 'fake', label: 'Simulado', provider: 'Simulado', key: 'fake', runs: 'cenário gravado, sem gastar tokens' },
];

export function cliInfo(cli: string): CliInfo {
  return CLIS.find((item) => item.id === cli) ?? { id: cli, label: cli, provider: cli, key: 'fake', runs: cli };
}

export const LOGIN_TYPES: Record<string, string> = {
  subscription: 'Assinatura',
  api_key: 'Chave de API',
};

export interface KindStyle {
  label: string;
  /** CSS class suffix: tag-edit… */
  tone: 'edit' | 'run' | 'msg' | 'read' | 'task' | 'alert' | 'wait';
}

const KINDS: Record<EventKind, KindStyle> = {
  status: { label: 'Status', tone: 'read' },
  thinking: { label: 'Pensou', tone: 'read' },
  file_read: { label: 'Leu', tone: 'read' },
  file_edit: { label: 'Editou', tone: 'edit' },
  command: { label: 'Rodou', tone: 'run' },
  command_result: { label: 'Resultado', tone: 'run' },
  message_sent: { label: 'Mensagem', tone: 'msg' },
  message_received: { label: 'Mensagem', tone: 'msg' },
  task_assigned: { label: 'Tarefa', tone: 'task' },
  task_done: { label: 'Concluiu', tone: 'task' },
  approval_requested: { label: 'Aprovação', tone: 'wait' },
  approval_decided: { label: 'Aprovação', tone: 'wait' },
  security_alert: { label: 'Alerta', tone: 'alert' },
  error: { label: 'Erro', tone: 'alert' },
};

export function kindStyle(kind: EventKind): KindStyle {
  return KINDS[kind] ?? { label: kind, tone: 'read' };
}

/** Title line of a timeline entry. */
export function eventTitle(event: AgentEvent): { text: string; mono: boolean } {
  switch (event.kind) {
    case 'file_read':
    case 'file_edit':
    case 'command':
      return { text: event.target || event.summary, mono: Boolean(event.target) };
    case 'message_sent':
      return { text: `Para ${event.target ?? 'outro agente'}`, mono: false };
    case 'message_received':
      return { text: `De ${event.target ?? 'outro agente'}`, mono: false };
    default:
      return { text: event.summary, mono: false };
  }
}

export const ACTION_LABELS: Record<string, string> = {
  merge: 'Merge',
  push: 'Push',
  command: 'Comando',
  delete: 'Apagar arquivos',
  migration: 'Migração de banco',
};

export const TASK_STATUS: Record<string, string> = {
  queued: 'na fila',
  running: 'em andamento',
  waiting: 'aguardando',
  blocked: 'bloqueada',
  done: 'feita',
  cancelled: 'cancelada',
};

/** Orchestrator first, then the others in the order of the daemon. */
export function sortTeam(agents: AgentView[]): AgentView[] {
  return [...agents].sort((a, b) => Number(b.is_orchestrator) - Number(a.is_orchestrator));
}
