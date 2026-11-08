import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Feed } from './Feed';
import { useChatStore } from '../stores/chat';

const json = (v: unknown, status = 200) =>
  new Response(JSON.stringify(v), { status, headers: { 'content-type': 'application/json' } });

const pending = { id: 'a1', tool: 'Bash', summary: 'Bash: rm -rf build' };

beforeEach(() => {
  useChatStore.setState({
    messages: { t1: [] },
    streaming: { t1: { text: 'working', tool: null } },
    approvals: { t1: [pending] },
  });
});

describe('approval row', () => {
  it('shows the summary with icon-only Allow and Deny buttons', () => {
    vi.stubGlobal('fetch', vi.fn());
    render(<Feed threadId="t1" />);
    expect(screen.getByText('Bash: rm -rf build')).toBeInTheDocument();
    const allow = screen.getByRole('button', { name: 'Allow' });
    const deny = screen.getByRole('button', { name: 'Deny' });
    expect(allow.textContent).toBe('');
    expect(deny.textContent).toBe('');
    expect(allow.querySelector('svg')).not.toBeNull();
    expect(deny.querySelector('svg')).not.toBeNull();
  });

  it('Allow posts the decision and the row disappears', async () => {
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    render(<Feed threadId="t1" />);
    await userEvent.click(screen.getByRole('button', { name: 'Allow' }));
    await waitFor(() => expect(screen.queryByText('Bash: rm -rf build')).toBeNull());
    const call = fetchMock.mock.calls.find(([u]) => u === '/api/threads/t1/approvals/a1') as unknown as [string, RequestInit];
    expect(call[1].method).toBe('POST');
    expect(JSON.parse(String(call[1].body))).toEqual({ allow: true });
  });

  it('Deny posts allow:false', async () => {
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    render(<Feed threadId="t1" />);
    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
    await waitFor(() => expect(screen.queryByText('Bash: rm -rf build')).toBeNull());
    const call = fetchMock.mock.calls.find(([u]) => u === '/api/threads/t1/approvals/a1') as unknown as [string, RequestInit];
    expect(JSON.parse(String(call[1].body))).toEqual({ allow: false });
  });

  it('renders nothing for a thread without requests', () => {
    useChatStore.setState({ approvals: {} });
    render(<Feed threadId="t1" />);
    expect(screen.queryByRole('button', { name: 'Allow' })).toBeNull();
  });
});

describe('chat store approvals', () => {
  it('loadApprovals fills the list from the server (reopened UI)', async () => {
    useChatStore.setState({ approvals: {} });
    vi.stubGlobal('fetch', vi.fn(async () => json([pending])));
    await useChatStore.getState().loadApprovals('t1');
    expect(useChatStore.getState().approvals['t1']).toEqual([pending]);
  });

  it('resolveApproval removes the request after a successful post', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 204 })));
    await useChatStore.getState().resolveApproval('t1', 'a1', true);
    expect(useChatStore.getState().approvals['t1'] ?? []).toEqual([]);
  });

  it('an approval event adds a row while streaming, and the list is empty after done', async () => {
    useChatStore.setState({ messages: {}, streaming: {}, approvals: {} });
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    const enc = new TextEncoder();
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (!url.endsWith('/messages')) return json([]);
      const body = new ReadableStream<Uint8Array>({
        async start(c) {
          c.enqueue(enc.encode(`data: ${JSON.stringify({ type: 'approval', ...pending })}\n\n`));
          await gate;
          c.enqueue(enc.encode('data: {"type":"done"}\n\n'));
          c.close();
        },
      });
      return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } });
    }));
    const p = useChatStore.getState().send('t1', 'go');
    await waitFor(() => expect(useChatStore.getState().approvals['t1']).toEqual([pending]));
    release();
    await p;
    expect(useChatStore.getState().approvals['t1'] ?? []).toEqual([]);
  });

  it('a duplicate approval event does not duplicate the row', async () => {
    useChatStore.setState({ messages: {}, streaming: {}, approvals: {} });
    const enc = new TextEncoder();
    const ev = `data: ${JSON.stringify({ type: 'approval', ...pending })}\n\n`;
    let seen = 0;
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (!url.endsWith('/messages')) return json([]);
      const body = new ReadableStream<Uint8Array>({
        start(c) {
          c.enqueue(enc.encode(ev + ev));
          c.enqueue(enc.encode('data: {"type":"done"}\n\n'));
          c.close();
        },
      });
      return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } });
    }));
    const unsub = useChatStore.subscribe((s) => {
      seen = Math.max(seen, s.approvals['t1']?.length ?? 0);
    });
    await useChatStore.getState().send('t1', 'go');
    unsub();
    expect(seen).toBe(1);
  });
});
