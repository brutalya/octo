import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useThreadsStore } from './threads';
import type { Thread } from '../api/types';

const thread: Thread = {
  id: 't1', title: '', provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
};
const json = (v: unknown) =>
  new Response(JSON.stringify(v), { status: 200, headers: { 'content-type': 'application/json' } });

beforeEach(() => useThreadsStore.setState({ threads: [thread], providers: [], activeId: null }));
afterEach(() => vi.unstubAllGlobals());

describe('threads store approval', () => {
  it('create sends approval', async () => {
    const f = vi.fn(async () => json({ ...thread, id: 't2', approval: 'auto' }));
    vi.stubGlobal('fetch', f);
    await useThreadsStore.getState().create('claude', 'public', 'auto');
    const init = (f.mock.calls[0] as unknown as [string, RequestInit])[1];
    expect(JSON.parse(init.body as string)).toEqual({ provider: 'claude', space: 'public', approval: 'auto' });
    expect(useThreadsStore.getState().activeId).toBe('t2');
  });

  it('setApproval PATCHes and updates the thread', async () => {
    const f = vi.fn(async () => json({ ...thread, approval: 'auto' }));
    vi.stubGlobal('fetch', f);
    await useThreadsStore.getState().setApproval('t1', 'auto');
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/api/threads/t1');
    expect(init.method).toBe('PATCH');
    expect(JSON.parse(init.body as string)).toEqual({ approval: 'auto' });
    expect(useThreadsStore.getState().threads[0].approval).toBe('auto');
  });
});

describe('threads store import', () => {
  it('imports a session, adds the thread and opens it', async () => {
    const f = vi.fn(async () => json({ ...thread, id: 'imp', origin: 'imported' }));
    vi.stubGlobal('fetch', f);
    await useThreadsStore.getState().importSession('claude', 'ext-1');
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/api/import/claude');
    expect(JSON.parse(init.body as string)).toEqual({ external_id: 'ext-1' });
    const s = useThreadsStore.getState();
    expect(s.activeId).toBe('imp');
    expect(s.threads.map((x) => x.id)).toEqual(['imp', 't1']);
  });

  it('an already imported session is not duplicated', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ ...thread, title: 'renamed' })));
    await useThreadsStore.getState().importSession('claude', 'ext-1');
    const s = useThreadsStore.getState();
    expect(s.threads).toHaveLength(1);
    expect(s.threads[0].title).toBe('renamed');
    expect(s.activeId).toBe('t1');
  });

  it('ignores a second create while one is pending', async () => {
    const f = vi.fn(async () => json({ ...thread, id: 'n' }));
    vi.stubGlobal('fetch', f);
    await Promise.all([
      useThreadsStore.getState().create('claude', 'public', 'confirm'),
      useThreadsStore.getState().create('claude', 'public', 'confirm'),
    ]);
    expect(f).toHaveBeenCalledTimes(1);
  });

  it('failures land in error instead of rejecting', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ error: 'nope' }), { status: 400 })));
    await expect(useThreadsStore.getState().remove('t1')).resolves.toBeUndefined();
    expect(useThreadsStore.getState().error).toBe('nope');
  });
});

describe('threads store keeps a thread open', () => {
  const other: Thread = { ...thread, id: 't2', updated_at: 2 };

  it('load opens the most recent thread when none is open', async () => {
    useThreadsStore.setState({ threads: [], activeId: null });
    vi.stubGlobal('fetch', vi.fn(async () => json([other, thread])));
    await useThreadsStore.getState().load();
    expect(useThreadsStore.getState().activeId).toBe('t2');
  });

  it('load keeps the open thread', async () => {
    useThreadsStore.setState({ activeId: 't1' });
    vi.stubGlobal('fetch', vi.fn(async () => json([other, thread])));
    await useThreadsStore.getState().load();
    expect(useThreadsStore.getState().activeId).toBe('t1');
  });

  it('load with no threads leaves nothing open', async () => {
    useThreadsStore.setState({ threads: [], activeId: null });
    vi.stubGlobal('fetch', vi.fn(async () => json([])));
    await useThreadsStore.getState().load();
    expect(useThreadsStore.getState().activeId).toBeNull();
  });

  it('deleting the open thread opens the next one', async () => {
    useThreadsStore.setState({ threads: [other, thread], activeId: 't2' });
    vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 204 })));
    await useThreadsStore.getState().remove('t2');
    expect(useThreadsStore.getState().activeId).toBe('t1');
  });

  it('deleting the last thread leaves nothing open', async () => {
    useThreadsStore.setState({ threads: [thread], activeId: 't1' });
    vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 204 })));
    await useThreadsStore.getState().remove('t1');
    expect(useThreadsStore.getState().activeId).toBeNull();
  });
});

describe('threads cache', () => {
  const priv: Thread = { ...thread, id: 'p1', title: 'Secret plan', space: 'private' };
  beforeEach(() => localStorage.clear());

  it('keeps the Public threads of this device, never Private ones', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json([thread, priv])));
    await useThreadsStore.getState().load();
    const raw = localStorage.getItem('octo.threads') ?? '';
    expect(JSON.parse(raw).map((x: Thread) => x.id)).toEqual(['t1']);
    expect(raw).not.toContain('Secret plan');
  });

  it('a list of another device is not cached as this device', async () => {
    const { setNodeBase } = await import('../api/client');
    setNodeBase('remote');
    try {
      vi.stubGlobal('fetch', vi.fn(async () => json([thread])));
      await useThreadsStore.getState().load();
      expect(localStorage.getItem('octo.threads')).toBeNull();
    } finally {
      setNodeBase(null);
    }
  });

  it('a new store starts from the cached list and its newest thread', async () => {
    localStorage.setItem('octo.threads', JSON.stringify([thread]));
    vi.resetModules();
    const fresh = await import('./threads');
    expect(fresh.useThreadsStore.getState().threads.map((x) => x.id)).toEqual(['t1']);
    expect(fresh.useThreadsStore.getState().activeId).toBe('t1');
  });

  it('ignores the cache while another device is the stored selection', async () => {
    localStorage.setItem('octo.threads', JSON.stringify([thread]));
    localStorage.setItem('octo.node', 'remote');
    vi.resetModules();
    const fresh = await import('./threads');
    expect(fresh.useThreadsStore.getState().threads).toEqual([]);
  });

  it('a broken cache is ignored', async () => {
    localStorage.setItem('octo.threads', '{oops');
    vi.resetModules();
    const fresh = await import('./threads');
    expect(fresh.useThreadsStore.getState().threads).toEqual([]);
  });
});
