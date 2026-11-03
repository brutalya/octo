import { readAgentEvents } from './sse';
import type {
  AgentEvent, Approval, Attachment, FileEntry, Message, ModelInfo, ProviderKind, ProviderStatus,
  Settings, SettingsPatch, Space, Thread,
} from './types';

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
  createThread: async (provider: ProviderKind, space: Space, approval: Approval, model?: string): Promise<Thread> =>
    (await request('/threads', jsonInit('POST', { provider, space, approval, model }))).json(),
  models: async (kind: ProviderKind): Promise<ModelInfo[]> => (await request(`/providers/${kind}/models`)).json(),
  settings: async (): Promise<Settings> => (await request('/settings')).json(),
  patchSettings: async (patch: SettingsPatch): Promise<Settings> =>
    (await request('/settings', jsonInit('PATCH', patch))).json(),
  files: async (space: Space): Promise<FileEntry[]> => (await request(`/files/${space}`)).json(),
  fileText: async (space: Space, path: string): Promise<string> =>
    (await request(`/files/${space}/content?path=${encodeURIComponent(path)}`)).text(),
  upload: async (space: Space, dir: string, file: File): Promise<void> => {
    const form = new FormData();
    form.append('file', file, file.name);
    await request(`/files/${space}?path=${encodeURIComponent(dir)}`, { method: 'POST', body: form });
  },
  mkdir: async (space: Space, path: string): Promise<void> => {
    await request(`/files/${space}/dir?path=${encodeURIComponent(path)}`, { method: 'POST' });
  },
  renameFile: async (space: Space, path: string, newPath: string): Promise<void> => {
    await request(`/files/${space}?path=${encodeURIComponent(path)}`, jsonInit('PATCH', { new_path: newPath }));
  },
  deleteFile: async (space: Space, path: string): Promise<void> => {
    await request(`/files/${space}?path=${encodeURIComponent(path)}`, { method: 'DELETE' });
  },
  setApproval: async (id: string, approval: Approval): Promise<Thread> =>
    (await request(`/threads/${id}`, jsonInit('PATCH', { approval }))).json(),
  deleteThread: async (id: string): Promise<void> => {
    await request(`/threads/${id}`, { method: 'DELETE' });
  },
  messages: async (id: string): Promise<Message[]> => (await request(`/threads/${id}/messages`)).json(),
  cancel: async (id: string): Promise<void> => {
    await request(`/threads/${id}/cancel`, { method: 'POST' });
  },
  async *send(
    id: string,
    content: string,
    signal: AbortSignal,
    attachments?: Attachment[],
  ): AsyncGenerator<AgentEvent> {
    const res = await request(`/threads/${id}/messages`, { ...jsonInit('POST', { content, attachments }), signal });
    if (!res.body) throw new ApiError(res.status, 'empty response body');
    yield* readAgentEvents(res.body);
  },
};
