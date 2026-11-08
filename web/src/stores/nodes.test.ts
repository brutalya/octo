import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useNodesStore } from './nodes';
import { useThreadsStore } from './threads';
import { useFilesStore } from './files';
import { useChatStore } from './chat';
import { api, setNodeBase } from '../api/client';

const nodes = [
  { node_id: 'me', name: 'Home', is_self: true, online: true, trust: null, agents: null },
  { node_id: 'vps', name: 'Vps', is_self: false, online: true, trust: 'public', agents: false },
];
let f: ReturnType<typeof vi.fn>;
beforeEach(() => {
  localStorage.clear();
  setNodeBase(null);
  useNodesStore.setState({ nodes: [], selectedId: null });
  f = vi.fn(async (url: string) => new Response(url.endsWith('/nodes') ? JSON.stringify(nodes) : '[]', { status: 200 }));
  vi.stubGlobal('fetch', f);
});
afterEach(() => vi.unstubAllGlobals());

describe('nodes store', () => {
  it('select remote re-targets api, persists and reloads threads', async () => {
    await useNodesStore.getState().load();
    useNodesStore.getState().select('vps');
    expect(localStorage.getItem('octo.node')).toBe('vps');
    await api.threads();
    expect(f.mock.calls.at(-1)?.[0]).toBe('/api/nodes/vps/threads');
    useNodesStore.getState().select('me');
    await api.threads();
    expect(f.mock.calls.at(-1)?.[0]).toBe('/api/threads');
  });

  it('restores stored selection once the node is known', async () => {
    localStorage.setItem('octo.node', 'vps');
    await useNodesStore.getState().load();
    expect(useNodesStore.getState().selectedId).toBe('vps');
    await api.threads();
    expect(f.mock.calls.at(-1)?.[0]).toBe('/api/nodes/vps/threads');
  });

  it('ignores a stored node that no longer exists', async () => {
    localStorage.setItem('octo.node', 'gone');
    await useNodesStore.getState().load();
    expect(useNodesStore.getState().selectedId).toBeNull();
  });

  it('offline selected node keeps data and reports offline', async () => {
    await useNodesStore.getState().load();
    useNodesStore.getState().select('vps');
    await new Promise((r) => setTimeout(r, 0));
    useThreadsStore.setState({ threads: [{ id: 'a' } as never] });
    f.mockImplementation(async () => new Response(JSON.stringify([nodes[0], { ...nodes[1], online: false }]), { status: 200 }));
    await useNodesStore.getState().load();
    expect(useNodesStore.getState().selectedOffline()).toBe(true);
    expect(useThreadsStore.getState().threads.length).toBe(1);
  });

  it('drops a stale threads load that resolves after a node switch', async () => {
    await useNodesStore.getState().load();
    let release: (r: Response) => void = () => {};
    f.mockImplementation(async (url: string) =>
      url === '/api/threads' ? new Promise<Response>((r) => (release = r)) : new Response('[]', { status: 200 }),
    );
    const p = useThreadsStore.getState().load();
    useNodesStore.getState().select('vps');
    release(new Response(JSON.stringify([{ id: 'stale' }]), { status: 200 }));
    await p;
    expect(useThreadsStore.getState().threads).toEqual([]);
  });

  it('drops a stale file preview after a node switch', async () => {
    await useNodesStore.getState().load();
    let release: (r: Response) => void = () => {};
    f.mockImplementation(async (url: string) =>
      url.includes('/content') ? new Promise<Response>((r) => (release = r)) : new Response('[]', { status: 200 }),
    );
    const p = useFilesStore.getState().open('public', 'a.md');
    useNodesStore.getState().select('vps');
    release(new Response('old', { status: 200 }));
    await p;
    expect(useFilesStore.getState().preview).toBeNull();
  });

  it('switching nodes clears chat state', async () => {
    await useNodesStore.getState().load();
    useChatStore.setState({ messages: { t: [] }, streaming: { t: { text: 'x', tool: null } } });
    useNodesStore.getState().select('vps');
    expect(useChatStore.getState().messages).toEqual({});
    expect(useChatStore.getState().streaming).toEqual({});
  });

  it('deselects a node that vanished from the list', async () => {
    await useNodesStore.getState().load();
    useNodesStore.getState().select('vps');
    f.mockImplementation(async () => new Response(JSON.stringify([nodes[0]]), { status: 200 }));
    await useNodesStore.getState().load();
    expect(useNodesStore.getState().selectedId).toBeNull();
    await api.threads();
    expect(f.mock.calls.at(-1)?.[0]).toBe('/api/threads');
  });

  it('does not overlap concurrent loads', async () => {
    const before = f.mock.calls.length;
    await Promise.all([useNodesStore.getState().load(), useNodesStore.getState().load()]);
    expect(f.mock.calls.slice(before).filter((c) => c[0] === '/api/nodes').length).toBe(1);
  });
});
