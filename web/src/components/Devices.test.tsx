import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Devices } from './Devices';

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });
const nodes = [
  { node_id: 'me', name: 'Home', is_self: true, online: true },
  { node_id: 'vps', name: 'Vps', is_self: false, online: true },
];
let f: ReturnType<typeof vi.fn>;
let pending: Response | null;
const settings = { ollama_url: '', openrouter_api_key: null, history_char_budget: 1, relay_url: 'ws://r/relay', node_name: 'Home' };
const calls = (m: string, u: string) =>
  f.mock.calls.filter((c) => c[0] === u && ((c[1] as RequestInit | undefined)?.method ?? 'GET') === m);

beforeEach(() => {
  pending = null;
  f = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/settings') return json(settings);
    if (url === '/api/nodes') return json(nodes);
    if (url === '/api/pairing/invite') return json({ code: 'ws://r/relay#TOK', expires_at: Math.floor(Date.now() / 1000) + 300 });
    if (url === '/api/pairing/pending') return pending ?? json({}, 404);
    if (url === '/api/pairing/join' && init?.method === 'POST') return json({}, 202);
    if (url.startsWith('/api/pairing/')) return json({});
    return json({}, 404);
  });
  vi.stubGlobal('fetch', f);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Devices', () => {
  it('shows node name, relay url and members', async () => {
    render(<Devices />);
    expect(await screen.findByLabelText('Node name')).toHaveValue('Home');
    expect(screen.getByLabelText('Relay URL')).toHaveValue('ws://r/relay');
    expect(await screen.findByText('Vps')).toBeInTheDocument();
  });

  it('saves the node name on blur', async () => {
    render(<Devices />);
    const name = await screen.findByLabelText('Node name');
    await userEvent.clear(name);
    await userEvent.type(name, 'Box');
    await userEvent.tab();
    const c = f.mock.calls.find((x) => (x[1] as RequestInit | undefined)?.method === 'PATCH');
    expect(JSON.parse(((c as unknown[])[1] as RequestInit).body as string)).toEqual({ node_name: 'Box' });
  });

  it('add device shows code with countdown, then SAS and confirm', async () => {
    render(<Devices />);
    await screen.findByLabelText('Node name');
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await act(async () => {
      screen.getByRole('button', { name: 'Add device' }).click();
    });
    expect(await screen.findByText('ws://r/relay#TOK')).toBeInTheDocument();
    expect(screen.getByText('5:00')).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(screen.getByText(/^4:5\d$/)).toBeInTheDocument();
    expect(calls('GET', '/api/pairing/pending').length).toBeGreaterThan(0);
    pending = json({ sas: '123456', peer_name: 'Phone' });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(screen.getByText('123456')).toBeInTheDocument();
    await act(async () => {
      screen.getByRole('button', { name: 'Confirm' }).click();
    });
    expect(calls('POST', '/api/pairing/confirm')).toHaveLength(1);
  });

  it('reject posts reject and clears the SAS', async () => {
    pending = json({ sas: '654321', peer_name: 'Phone' });
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    expect(await screen.findByText('654321', {}, { timeout: 4000 })).toBeInTheDocument();
    pending = null;
    await userEvent.click(screen.getByRole('button', { name: 'Reject' }));
    expect(calls('POST', '/api/pairing/reject')).toHaveLength(1);
    await waitFor(() => expect(screen.queryByText('654321')).toBeNull());
  });

  it('join sends code and name, then shows SAS to confirm', async () => {
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    await userEvent.type(screen.getByLabelText('Code'), 'ws://r/relay#TOK');
    pending = json({ sas: '111222', peer_name: 'hostid' });
    await userEvent.click(screen.getByRole('button', { name: 'Connect' }));
    const join = calls('POST', '/api/pairing/join')[0];
    expect(JSON.parse((join[1] as RequestInit).body as string)).toEqual({ code: 'ws://r/relay#TOK', name: 'Home' });
    expect(await screen.findByText('111222', {}, { timeout: 4000 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(calls('POST', '/api/pairing/confirm')).toHaveLength(1);
  });
});
