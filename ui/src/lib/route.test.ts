import { describe, expect, it } from 'vitest';
import { parseRoute, routeHash, type Route } from './route';

describe('hash routes', () => {
  it.each<Route>([
    { screen: 'overview' }, { screen: 'overview', projectId: 'projeto / 1' },
    { screen: 'agent', agentId: 'agente / 1' }, { screen: 'profiles' },
  ])('round trips $screen', (route) => expect(parseRoute(routeHash(route))).toEqual(route));
  it('falls back to overview for an unknown route or missing agent', () => {
    expect(parseRoute('#/inexistente')).toEqual({ screen: 'overview' });
    expect(parseRoute('#/agente')).toEqual({ screen: 'overview' });
  });
});
