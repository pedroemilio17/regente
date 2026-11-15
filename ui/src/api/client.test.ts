import { describe, expect, it, vi } from 'vitest';
import { api, ApiError, apiUrl, wsUrl } from './client';

describe('daemon client', () => {
  it('encodes query and resource identifiers', () => {
    expect(apiUrl('/agents', { project: 'a / b', limit: 0, active: false, skip: null })).toBe('/api/v1/agents?project=a+%2F+b&limit=0&active=false');
    expect(wsUrl('/stream', { protocol: 'https:', host: 'example.test' })).toBe('wss://example.test/api/v1/stream');
  });
  it('sends decisions as JSON and exposes daemon errors', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(JSON.stringify({ error: 'negado' }), { status: 403 }));
    await expect(api.decideApproval('a/b', 'deny')).rejects.toMatchObject({ name: 'ApiError', status: 403, message: 'negado' } satisfies Partial<ApiError>);
    expect(fetchMock).toHaveBeenCalledWith('/api/v1/approvals/a%2Fb', expect.objectContaining({ method: 'POST', body: '{"decision":"deny"}' }));
  });
});
