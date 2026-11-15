// Formatting helpers. Every string here is shown to the user (pt-BR).

import type { UsageWindow, UsageWindows, WindowKey } from '../api/types';

/** `0.94` -> `US$ 0,94`. */
export function formatUsd(value: number | null | undefined): string {
  const amount = Number.isFinite(value) ? (value as number) : 0;
  return `US$ ${amount.toFixed(2).replace('.', ',')}`;
}

/** `141000` -> `141 mil`, `1500000` -> `1,5 mi`. */
export function formatTokens(tokens: number | null | undefined): string {
  const count = Number.isFinite(tokens) ? Math.max(0, tokens as number) : 0;
  if (count >= 1_000_000) {
    return `${(count / 1_000_000).toFixed(1).replace('.', ',').replace(',0', '')} mi`;
  }
  if (count >= 1000) {
    return `${Math.round(count / 1000)} mil`;
  }
  return String(Math.round(count));
}

/** Windows with a quota reported by the CLI. */
export const WINDOW_ORDER: WindowKey[] = ['5h', 'week'];

export const WINDOW_LABELS: Record<WindowKey, string> = { '5h': '5h', day: 'Dia', week: 'Semana' };

export interface UsageBar {
  key: WindowKey;
  label: string;
  /** `62% disponível`, or `sem dados de cota` when the CLI reported nothing. */
  value: string;
  /** Available share in percent (0..100); null without data. The bar shows what is left. */
  width: number | null;
  /** `reinicia às 14:20`, empty when unknown. */
  reset: string;
  costUsd: number;
}

/** `reinicia às 14:20`; other days get the weekday. Empty for a missing or invalid timestamp. */
export function formatReset(resetsAt: string | null | undefined): string {
  const time = formatTime(resetsAt);
  if (!time) {
    return '';
  }
  const date = new Date(resetsAt as string);
  if (date.toDateString() === new Date().toDateString()) {
    return `reinicia às ${time}`;
  }
  return `reinicia ${date.toLocaleDateString('pt-BR', { weekday: 'short' }).replace('.', '')} ${time}`;
}

/** One bar: available share (100 - used) when the CLI reported it, no invented numbers otherwise. */
export function usageBar(key: WindowKey, window: UsageWindow | undefined): UsageBar {
  const costUsd = window?.cost_usd ?? 0;
  const pct = window?.pct;
  if (typeof pct === 'number' && Number.isFinite(pct)) {
    const available = Math.round(Math.min(100, Math.max(0, (1 - pct) * 100)));
    return { key, label: WINDOW_LABELS[key], value: `${available}% disponível`, width: available, reset: formatReset(window?.resets_at), costUsd };
  }
  return { key, label: WINDOW_LABELS[key], value: 'sem dados de cota', width: null, reset: '', costUsd };
}

export function usageBars(windows: UsageWindows | undefined): UsageBar[] {
  return WINDOW_ORDER.map((key) => usageBar(key, windows?.[key]));
}

/** `14:13` in the local time zone; empty for a missing or invalid timestamp. */
export function formatTime(ts: string | null | undefined): string {
  if (!ts) {
    return '';
  }
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) {
    return '';
  }
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

/** `há 6 min`, relative to `now`. */
export function formatAgo(ts: string | null | undefined, now: number = Date.now()): string {
  if (!ts) {
    return '';
  }
  const then = new Date(ts).getTime();
  if (Number.isNaN(then)) {
    return '';
  }
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) {
    return 'agora';
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `há ${minutes} min`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 24) {
    return `há ${hours} h`;
  }
  const days = Math.round(hours / 24);
  return `há ${days} ${days === 1 ? 'dia' : 'dias'}`;
}

/** Parse a money field typed in pt-BR (`2,50`) or with a dot; null when empty or invalid. */
export function parseUsd(input: string): number | null {
  const clean = input.trim().replace(/\s|US\$/gi, '').replace(',', '.');
  if (clean === '') {
    return null;
  }
  const value = Number(clean);
  return Number.isFinite(value) && value >= 0 ? value : null;
}
