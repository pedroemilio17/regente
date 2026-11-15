import type { AccountStatus } from '../api/types';

/** Badge of each login state (D21); `tone` reuses the status pill colors. */
export const ACCOUNT_STATES: Record<AccountStatus['state'], { label: string; tone: string }> = {
  connected: { label: 'Conectado', tone: 'working' },
  disconnected: { label: 'Não conectado', tone: 'paused' },
  unknown: { label: 'Desconhecido', tone: 'idle' },
};

/** Variable each CLI reads when the account logs in with an API key. */
export const API_KEY_ENV: Record<string, string> = {
  claude: 'ANTHROPIC_API_KEY',
  codex: 'OPENAI_API_KEY',
  agy: 'GEMINI_API_KEY',
};
