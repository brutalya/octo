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
    expect(urls.sort()).toEqual(['/api/nodes/pc/access', '/api/nodes/pc/threads', '/api/threads']);
    const lists = useDevicesStore.getState().lists;
    expect(lists.me.map((t) => t.id)).toEqual(['mine']);
    expect(lists.pc.map((t) => t.id)).toEqual(['on-pc']);
    expect(lists.lap.map((t) => t.id)).toEqual(['cached']);
  });

  it('asks each online member what this device may do there', async () => {
    f.mockImplementation(async (url: string) => {
      if (url === '/api/nodes/pc/access') return new Response(JSON.stringify({ agents: true, private_files: false }));
      if (url === '/api/nodes/vps/access') return new Response(JSON.stringify({ agents: false, private_files: false }));
      return new Response('[]');
    });
    useNodesStore.setState({
      nodes: [
        { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
        { node_id: 'pc', name: 'pc', is_self: false, online: true, trust: 'full', agents: true },
        { node_id: 'vps', name: 'vps', is_self: false, online: true, trust: 'public', agents: false },
      ],
    });
    useDevicesStore.setState({ access: {} });
    await useDevicesStore.getState().load();
    expect(useDevicesStore.getState().access).toEqual({
      pc: { agents: true, private_files: false },
      vps: { agents: false, private_files: false },
    });
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

describe('access is asked rarely', () => {
  const pc = { node_id: 'pc', name: 'pc', is_self: false, online: true, trust: 'full' as const, agents: true };
  const asked = () => f.mock.calls.filter((c) => c[0] === '/api/nodes/pc/access').length;

  beforeEach(() => {
    f.mockImplementation(async (url: string) =>
      new Response(url.endsWith('/access') ? JSON.stringify({ agents: true, private_files: false }) : '[]'),
    );
    useDevicesStore.setState({ lists: {}, access: {}, accessAt: {} });
  });

  it('once per 5 minutes while a device stays online; again when it comes back', async () => {
    vi.useFakeTimers({ now: 1_000_000 });
    try {
      console.log('[Step 1/3] first load asks, the 20 s refreshes do not');
      useNodesStore.setState({ nodes: [pc] });
      await useDevicesStore.getState().load();
      await useDevicesStore.getState().load();
      vi.setSystemTime(1_000_000 + 60_000);
      await useDevicesStore.getState().load();
      expect(asked()).toBe(1);

      console.log('[Step 2/3] after 5 minutes it asks again');
      vi.setSystemTime(1_000_000 + 5 * 60_000 + 1);
      await useDevicesStore.getState().load();
      expect(asked()).toBe(2);

      console.log('[Step 3/3] offline, then back online: asked right away');
      useNodesStore.setState({ nodes: [{ ...pc, online: false }] });
      await useDevicesStore.getState().load();
      useNodesStore.setState({ nodes: [pc] });
      await useDevicesStore.getState().load();
      expect(asked()).toBe(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it('a device too old to answer is listed as before (its own checks still apply)', async () => {
    console.log('[Step 1/2] the pc runs a version without /api/access: the tunnel answers 403');
    useNodesStore.setState({
      nodes: [
        { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
        { node_id: 'old', name: 'pc', is_self: false, online: true, trust: 'full', agents: true },
        { node_id: 'gone', name: 'laptop', is_self: false, online: true, trust: 'full', agents: true },
      ],
    });
    useDevicesStore.setState({ access: {}, accessAt: {} });
    f.mockImplementation(async (url: string) => {
      if (url === '/api/nodes/old/access') return new Response('{"error":"not available through the tunnel"}', { status: 403 });
      return new Response('{"error":"node unreachable"}', { status: 502 });
    });
    await useDevicesStore.getState().load();
    console.log('[Step 2/2] old: agents assumed allowed; unreachable: still unknown (asked again later)');
    expect(useDevicesStore.getState().access.old).toEqual({ agents: true, private_files: false });
    expect(useDevicesStore.getState().access.gone).toBeUndefined();
  });
});

