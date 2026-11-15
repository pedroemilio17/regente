// Minimal hash router: no dependency, one subscription.

import { useSyncExternalStore } from 'react';

export type Route =
  | { screen: 'overview'; projectId?: string }
  | { screen: 'agent'; agentId: string }
  | { screen: 'profiles' };

export function parseRoute(hash: string): Route {
  let parts: string[];
  try {
    parts = hash
    .replace(/^#\/?/, '')
    .split('/')
    .filter(Boolean)
    .map((part) => decodeURIComponent(part));
  } catch {
    return { screen: 'overview' };
  }
  switch (parts[0]) {
    case 'agente':
      return parts[1] ? { screen: 'agent', agentId: parts[1] } : { screen: 'overview' };
    case 'projeto':
      return { screen: 'overview', projectId: parts[1] };
    case 'perfis':
      return { screen: 'profiles' };
    default:
      return { screen: 'overview' };
  }
}

export function routeHash(route: Route): string {
  switch (route.screen) {
    case 'agent':
      return `#/agente/${encodeURIComponent(route.agentId)}`;
    case 'profiles':
      return '#/perfis';
    default:
      return route.projectId ? `#/projeto/${encodeURIComponent(route.projectId)}` : '#/';
  }
}

export function navigate(route: Route): void {
  window.location.hash = routeHash(route);
}

function subscribe(listener: () => void): () => void {
  window.addEventListener('hashchange', listener);
  return () => window.removeEventListener('hashchange', listener);
}

const currentHash = () => window.location.hash;

export function useHash(): string {
  return useSyncExternalStore(subscribe, currentHash);
}
