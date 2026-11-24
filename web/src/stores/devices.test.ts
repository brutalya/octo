import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useDevicesStore } from './devices';
import { usePinsStore, pinKey } from './pins';
import { useNodesStore } from './nodes';
import { setNodeBase } from '../api/client';

const thread = (id: string) => ({
  id, title: id, provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
});
let f: ReturnType<typeof vi.fn>;

beforeEach(() => {
  localStorage.clear();
  setNodeBase('pc');
  useDevicesStore.setState({ lists: {} });
  f = vi.fn(async (url: string) => {
    if (url === '/api/threads') return new Response(JSON.stringify([thread('mine')]));
    if (url === '/api/nodes/pc/threads') return new Response(JSON.stringify([thread('on-pc')]));
    return new Response('nope', { status: 502 });
  });
  vi.stubGlobal('fetch', f);
});
afterEach(() => {
  vi.unstubAllGlobals();
  setNodeBase(null);
});

describe('devices store', () => {
  it('loads every reachable device on its own base, whatever device is open', async () => {
    console.log('[Step 1/2] this device, an online pc and an offline laptop; the pc is open');
    useNodesStore.setState({
      nodes: [
        { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
        { node_id: 'pc', name: 'pc', is_self: false, online: true, trust: 'full', agents: true },
        { node_id: 'lap', name: 'laptop', is_self: false, online: false, trust: 'full', agents: true },
      ],
    });
    useDevicesStore.setState({ lists: { lap: [thread('cached') as never] } });
    await useDevicesStore.getState().load();
    console.log('[Step 2/2] own list from /api, the pc through the tunnel, the laptop keeps its last list');
    const urls = f.mock.calls.map((c) => c[0]);
    expect(urls.sort()).toEqual(['/api/nodes/pc/threads', '/api/threads']);
    const lists = useDevicesStore.getState().lists;
    expect(lists.me.map((t) => t.id)).toEqual(['mine']);
    expect(lists.pc.map((t) => t.id)).toEqual(['on-pc']);
    expect(lists.lap.map((t) => t.id)).toEqual(['cached']);
  });

  it('a device that fails keeps its last list', async () => {
    useNodesStore.setState({
      nodes: [{ node_id: 'vps', name: 'vps', is_self: false, online: true, trust: 'public', agents: false }],
    });
    useDevicesStore.setState({ lists: { vps: [thread('old') as never] } });
    await useDevicesStore.getState().load();
    expect(useDevicesStore.getState().lists.vps.map((t) => t.id)).toEqual(['old']);
  });
});

describe('pins store', () => {
  it('pins per device and thread, survives a reload', () => {
    usePinsStore.setState({ pins: [] });
    usePinsStore.getState().toggle(pinKey('pc', 't1'));
    expect(usePinsStore.getState().pins).toEqual(['pc/t1']);
    expect(JSON.parse(localStorage.getItem('octo.pins') ?? '[]')).toEqual(['pc/t1']);
    usePinsStore.getState().toggle(pinKey('pc', 't1'));
    expect(usePinsStore.getState().pins).toEqual([]);
  });
});
