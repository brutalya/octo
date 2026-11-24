import { readAgentEvents } from './sse';
import type {
  AccessView, AgentEvent, Approval, AttachmentRef, FileEntry, Message, ModelInfo, NetworkView, NodeInfo, PairingInvite, PairingPending, PairingState, RelayCheck, RelayStatus, PendingApproval, ProviderKind, ProviderStatus,
  SessionSummary, SettingsView, SettingsPatch, Space, Thread, TrustLevel,
} from './types';

const LOCAL = '/api';
let base = LOCAL;
let generation = 0;

/** Re-target data calls to a remote node; null = this node. */
export function setNodeBase(nodeId: string | null): void {
  const next = baseOf(nodeId);
  if (next === base) return;
  base = next;
  generation++;
}

/** Bumps on every node switch; loads drop results whose captured value is stale. */
export const nodeGeneration = (): number => generation;

/** An HTTP error response: the server refused the request (for a send: nothing was stored). */
export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

function errorMessage(text: string, status: number): string {
  let msg = text;
  try {
    const j: unknown = JSON.parse(text);
    if (typeof j === 'object' && j !== null && typeof (j as { error?: unknown }).error === 'string') {
      msg = (j as { error: string }).error;
    }
  } catch {
    // plain text body
  }
  return (msg || `HTTP ${status}`).slice(0, 300);
}

/** Base for a device's data calls: this device (null) or a member, through the tunnel. */
const baseOf = (nodeId: string | null): string =>
  nodeId ? `${LOCAL}/nodes/${encodeURIComponent(nodeId)}` : LOCAL;

async function request(path: string, init?: RequestInit, local = false): Promise<Response> {
  return requestAt(local ? LOCAL : base, path, init);
}

async function requestAt(prefix: string, path: string, init?: RequestInit): Promise<Response> {
  const token = (window as { __OCTO_TOKEN__?: string }).__OCTO_TOKEN__;
  if (token) {
    const headers = new Headers(init?.headers);
    headers.set('authorization', `Bearer ${token}`);
    init = { ...init, headers };
  }
  const res = await fetch(prefix + path, init);
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new ApiError(res.status, errorMessage(text, res.status));
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
  /** Assistants installed on a given device (null = this one), whatever device is open. */
  providersOn: async (nodeId: string | null): Promise<ProviderStatus[]> =>
    (await requestAt(baseOf(nodeId), '/providers')).json(),
  importSessionsOn: async (nodeId: string | null, kind: ProviderKind, q: string): Promise<SessionSummary[]> =>
    (await requestAt(baseOf(nodeId), `/import/${kind}/sessions?q=${encodeURIComponent(q)}`)).json(),
  /** What this device may do on a member (asked through the tunnel). */
  accessOn: async (nodeId: string): Promise<AccessView> => (await requestAt(baseOf(nodeId), '/access')).json(),
  /** Threads of a given device (null = this one), whatever device is open. */
  threadsOn: async (nodeId: string | null): Promise<Thread[]> =>
    (await requestAt(baseOf(nodeId), '/threads')).json(),
  deleteThreadOn: async (nodeId: string | null, id: string): Promise<void> => {
    await requestAt(baseOf(nodeId), `/threads/${id}`, { method: 'DELETE' });
  },
  createThread: async (provider: ProviderKind, space: Space, approval: Approval, model?: string): Promise<Thread> =>
    (await request('/threads', jsonInit('POST', { provider, space, approval, model }))).json(),
  importSessions: async (kind: ProviderKind, q: string): Promise<SessionSummary[]> =>
    (await request(`/import/${kind}/sessions?q=${encodeURIComponent(q)}`)).json(),
  importSession: async (kind: ProviderKind, externalId: string): Promise<Thread> =>
    (await request(`/import/${kind}`, jsonInit('POST', { external_id: externalId }))).json(),
  models: async (kind: ProviderKind): Promise<ModelInfo[]> => (await request(`/providers/${kind}/models`)).json(),
  settings: async (): Promise<SettingsView> => (await request('/settings', undefined, true)).json(),
  patchSettings: async (patch: SettingsPatch): Promise<SettingsView> =>
    (await request('/settings', jsonInit('PATCH', patch), true)).json(),
  nodes: async (): Promise<NodeInfo[]> => {
    const n: unknown = await (await request('/nodes', undefined, true)).json();
    return Array.isArray(n) ? (n as NodeInfo[]) : [];
  },
  pairingInvite: async (): Promise<PairingInvite> =>
    (await request('/pairing/invite', { method: 'POST' }, true)).json(),
  pairingJoin: async (code: string, name: string): Promise<void> => {
    await request('/pairing/join', jsonInit('POST', { code, name }), true);
  },
  /** Where this device's pairing stands (Settings shows a running invite again). */
  pairingState: async (): Promise<PairingState> => (await request('/pairing', undefined, true)).json(),
  /** Ends the running invite or join; its code stops working. */
  pairingCancel: async (): Promise<void> => {
    await request('/pairing/cancel', { method: 'POST' }, true);
  },
  /** This device's network (local only, like settings). */
  network: async (): Promise<NetworkView> => (await request('/network', undefined, true)).json(),
  /** Starts a network on the relay of a relay code, this device first. */
  networkCreate: async (relayUrl: string, name: string): Promise<NetworkView> =>
    (await request('/network/create', jsonInit('POST', { relay_url: relayUrl, name }), true)).json(),
  /** Forgets the other devices, the relay and the network name. */
  networkLeave: async (): Promise<void> => {
    await request('/network/leave', { method: 'POST' }, true);
  },
  /** This device's relay link; never routed to a remote node (the tunnel refuses it). */
  relayStatus: async (): Promise<RelayStatus> => (await request('/relay', undefined, true)).json(),
  relayCheck: async (relayUrl?: string): Promise<RelayCheck> =>
    (await request('/relay/check', jsonInit('POST', relayUrl ? { relay_url: relayUrl } : {}), true)).json(),
  /** null when no pairing is pending (404). */
  pairingPending: async (): Promise<PairingPending | null> => {
    try {
      return await (await request('/pairing/pending', undefined, true)).json();
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  },
  pairingConfirm: async (trustPrivate = false, allowAgents = false): Promise<void> => {
    await request('/pairing/confirm', jsonInit('POST', { trust_private: trustPrivate, allow_agents: allowAgents }), true);
  },
  setNodeAgents: async (id: string, agents: boolean): Promise<void> => {
    await request(`/nodes/${encodeURIComponent(id)}/agents`, jsonInit('PATCH', { agents }), true);
  },
  setNodeTrust: async (id: string, trust: TrustLevel): Promise<void> => {
    await request(`/nodes/${encodeURIComponent(id)}/trust`, jsonInit('PATCH', { trust }), true);
  },
  removeNode: async (id: string): Promise<void> => {
    await request(`/nodes/${encodeURIComponent(id)}`, { method: 'DELETE' }, true);
  },
  pairingReject: async (): Promise<void> => {
    await request('/pairing/reject', { method: 'POST' }, true);
  },
  files: async (space: Space): Promise<FileEntry[]> => (await request(`/files/${space}`)).json(),
  fileText: async (space: Space, path: string): Promise<string> =>
    (await request(`/files/${space}/content?path=${encodeURIComponent(path)}`)).text(),
  fileBlob: async (space: Space, path: string): Promise<Blob> =>
    (await request(`/files/${space}/content?path=${encodeURIComponent(path)}`)).blob(),
  /** Returns the stored entries (their paths are what attachments refer to). */
  upload: async (space: Space, dir: string, file: File): Promise<FileEntry[]> => {
    const form = new FormData();
    form.append('file', file, file.name);
    return (await request(`/files/${space}?path=${encodeURIComponent(dir)}`, { method: 'POST', body: form })).json();
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
  approvals: async (id: string): Promise<PendingApproval[]> => (await request(`/threads/${id}/approvals`)).json(),
  resolveApproval: async (id: string, aid: string, allow: boolean): Promise<void> => {
    await request(`/threads/${id}/approvals/${aid}`, jsonInit('POST', { allow }));
  },
  async *send(
    id: string,
    content: string,
    signal: AbortSignal,
    attachments?: AttachmentRef[],
  ): AsyncGenerator<AgentEvent> {
    const res = await request(`/threads/${id}/messages`, { ...jsonInit('POST', { content, attachments }), signal });
    if (!res.body) throw new ApiError(res.status, 'empty response body');
    yield* readAgentEvents(res.body);
  },
};
