import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useNodesStore } from './nodes';
import { useThreadsStore } from './threads';
import { api, setNodeBase } from '../api/client';

const nodes = [
  { node_id: 'me', name: 'Home', is_self: true, online: true },
  { node_id: 'vps', name: 'Vps', is_self: false, online: true },
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
});
