import { readAgentEvents } from './sse';
import type { AgentEvent, Approval, Message, ProviderKind, ProviderStatus, Space, Thread } from './types';

const BASE = '/api';

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(BASE + path, init);
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new ApiError(res.status, text || `HTTP ${res.status}`);
  }
  return res;
}

const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
});

export const api = {
  providers: async (): Promise<ProviderStatus[]> => (await request('/providers')).json(),
  threads: async (): Promise<Thread[]> => (await request('/threads')).json(),
  createThread: async (provider: ProviderKind, space: Space, approval: Approval): Promise<Thread> =>
    (await request('/threads', jsonInit('POST', { provider, space, approval }))).json(),
  setApproval: async (id: string, approval: Approval): Promise<Thread> =>
    (await request(`/threads/${id}`, jsonInit('PATCH', { approval }))).json(),
  deleteThread: async (id: string): Promise<void> => {
    await request(`/threads/${id}`, { method: 'DELETE' });
  },
  messages: async (id: string): Promise<Message[]> => (await request(`/threads/${id}/messages`)).json(),
  cancel: async (id: string): Promise<void> => {
    await request(`/threads/${id}/cancel`, { method: 'POST' });
  },
  async *send(id: string, content: string, signal: AbortSignal): AsyncGenerator<AgentEvent> {
    const res = await request(`/threads/${id}/messages`, { ...jsonInit('POST', { content }), signal });
    if (!res.body) throw new ApiError(res.status, 'empty response body');
    yield* readAgentEvents(res.body);
  },
};
