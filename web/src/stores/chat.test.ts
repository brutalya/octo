import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useChatStore } from './chat';
import { useThreadsStore } from './threads';
import type { Message, Thread } from '../api/types';

function sse(...events: object[]): Response {
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      for (const e of events) c.enqueue(enc.encode(`data: ${JSON.stringify(e)}\n\n`));
      c.close();
    },
  });
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } });
}

const json = (v: unknown, status = 200) =>
  new Response(JSON.stringify(v), { status, headers: { 'content-type': 'application/json' } });

const thread: Thread = {
  id: 't1', title: '', provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
};

beforeEach(() => {
  useChatStore.setState({ messages: {}, streaming: {} });
  useThreadsStore.setState({ threads: [thread], providers: [], activeId: 't1' });
});

describe('chat store', () => {
  it('send streams deltas then persists assistant message', async () => {
    const fetchMock = vi.fn(async (...args: [string, RequestInit?]) => {
      const url = args[0];
      if (url.endsWith('/messages') ) {
        return sse(
          { type: 'session', id: 's' },
          { type: 'text_delta', text: 'Hel' },
          { type: 'text_delta', text: 'lo' },
          { type: 'done' },
        );
      }
      return json([thread]); // threads refresh
    });
    vi.stubGlobal('fetch', fetchMock);

    await useChatStore.getState().send('t1', 'hi');

    const msgs = useChatStore.getState().messages['t1'];
    expect(msgs.map((m) => [m.role, m.content])).toEqual([
      ['user', 'hi'],
      ['assistant', 'Hello'],
    ]);
    expect(useChatStore.getState().streaming['t1']).toBeUndefined();
    const call = fetchMock.mock.calls.find(([u]) => u === '/api/threads/t1/messages');
    expect(call).toBeDefined();
    expect(JSON.parse(String(call?.[1]?.body))).toEqual({ content: 'hi' });
  });

  it('exposes partial text while streaming', async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    const enc = new TextEncoder();
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (!url.endsWith('/messages')) return json([thread]);
      const body = new ReadableStream<Uint8Array>({
        async start(c) {
          c.enqueue(enc.encode('data: {"type":"text_delta","text":"par"}\n\n'));
          await gate;
          c.enqueue(enc.encode('data: {"type":"done"}\n\n'));
          c.close();
        },
      });
      return new Response(body);
    }));
    const p = useChatStore.getState().send('t1', 'x');
    await vi.waitFor(() => expect(useChatStore.getState().streaming['t1']?.text).toBe('par'));
    release();
    await p;
    expect(useChatStore.getState().messages['t1'].at(-1)?.content).toBe('par');
  });

  it('error event persists as error message', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) =>
      url.endsWith('/messages')
        ? sse({ type: 'text_delta', text: 'partial' }, { type: 'error', message: 'boom' })
        : json([thread]),
    ));
    await useChatStore.getState().send('t1', 'hi');
    const msgs = useChatStore.getState().messages['t1'];
    expect(msgs.at(-1)).toMatchObject({ role: 'error', content: 'boom' });
    expect(useChatStore.getState().streaming['t1']).toBeUndefined();
  });

  it('http failure becomes an error message', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('busy', { status: 409 })));
    await useChatStore.getState().send('t1', 'hi');
    expect(useChatStore.getState().messages['t1'].at(-1)?.role).toBe('error');
    expect(useChatStore.getState().streaming['t1']).toBeUndefined();
  });

  it('stop aborts the request, posts cancel and reloads history', async () => {
    const history: Message[] = [
      { id: 'm1', thread_id: 't1', role: 'user', content: 'hi', attachments: [], created_at: 1 },
    ];
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url.endsWith('/messages') && init?.method === 'POST') {
        return new Promise<Response>((_, reject) => {
          init.signal?.addEventListener('abort', () =>
            reject(new DOMException('aborted', 'AbortError')),
          );
        });
      }
      if (url.endsWith('/cancel')) return new Response(null, { status: 204 });
      if (url.endsWith('/messages')) return json(history);
      return json([thread]);
    });
    vi.stubGlobal('fetch', fetchMock);

    const p = useChatStore.getState().send('t1', 'hi');
    await vi.waitFor(() => expect(useChatStore.getState().streaming['t1']).toBeDefined());
    await useChatStore.getState().stop('t1');
    await p;

    expect(fetchMock.mock.calls.some(([u]) => u === '/api/threads/t1/cancel')).toBe(true);
    expect(useChatStore.getState().streaming['t1']).toBeUndefined();
    expect(useChatStore.getState().messages['t1']).toEqual(history);
  });

  it('load fetches messages for a thread', async () => {
    const m: Message = { id: 'm', thread_id: 't1', role: 'assistant', content: 'yo', attachments: [], created_at: 2 };
    vi.stubGlobal('fetch', vi.fn(async () => json([m])));
    await useChatStore.getState().load('t1');
    expect(useChatStore.getState().messages['t1']).toEqual([m]);
  });
});

describe('threads store', () => {
  it('create adds thread on top and activates it', async () => {
    const created: Thread = { ...thread, id: 't2', provider: 'antigravity' };
    const fetchMock = vi.fn(async () => json(created));
    vi.stubGlobal('fetch', fetchMock);
    await useThreadsStore.getState().create('antigravity', 'public', 'confirm');
    const s = useThreadsStore.getState();
    expect(s.threads.map((t) => t.id)).toEqual(['t2', 't1']);
    expect(s.activeId).toBe('t2');
    expect(JSON.parse(String((fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].body)))
      .toEqual({ provider: 'antigravity', space: 'public', approval: 'confirm' });
  });

  it('remove deletes and clears active', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 204 })));
    await useThreadsStore.getState().remove('t1');
    expect(useThreadsStore.getState().threads).toEqual([]);
    expect(useThreadsStore.getState().activeId).toBeNull();
  });

  it('loadProviders keeps only installed', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json([
      { kind: 'claude', installed: true, version: '1', is_local: false },
      { kind: 'antigravity', installed: false, version: null, is_local: false },
    ])));
    await useThreadsStore.getState().loadProviders();
    expect(useThreadsStore.getState().providers.map((p) => p.kind)).toEqual(['claude']);
  });
});
