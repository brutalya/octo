import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useThreadsStore } from './threads';
import type { Thread } from '../api/types';

const thread: Thread = {
  id: 't1', title: '', provider: 'claude', space: 'public', approval: 'confirm', model: null, created_at: 1, updated_at: 1,
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
