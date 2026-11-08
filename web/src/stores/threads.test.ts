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
