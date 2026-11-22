import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Devices } from './Devices';

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });
const nodes = [
  { node_id: 'me', name: 'Home', is_self: true, online: true, trust: null, agents: null },
  { node_id: 'vps', name: 'Vps', is_self: false, online: true, trust: 'public', agents: false },
  { node_id: 'pc', name: 'Pc', is_self: false, online: true, trust: 'full', agents: true },
];
let f: ReturnType<typeof vi.fn>;
let pending: Response | null;
let relayStatus: unknown;
let relayCheck: unknown;
const settings = { ollama_url: '', openrouter_api_key: null, history_char_budget: 1, relay_url: 'ws://r/relay', node_name: 'Home' };
const calls = (m: string, u: string) =>
  f.mock.calls.filter((c) => c[0] === u && ((c[1] as RequestInit | undefined)?.method ?? 'GET') === m);

beforeEach(() => {
  pending = null;
  relayStatus = { configured: true, active: true, online: true, error: null };
  relayCheck = { ok: true, error: null };
  f = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/settings') return json(settings);
    if (url === '/api/nodes') return json(nodes);
    if (url === '/api/pairing/invite') return json({ code: 'ws://r/relay#TOK', expires_at: Math.floor(Date.now() / 1000) + 300 });
    if (url === '/api/pairing/pending') return pending ?? json({}, 404);
    if (url === '/api/pairing/join' && init?.method === 'POST') return json({}, 202);
    if (url === '/api/relay') return json(relayStatus);
    if (url === '/api/relay/check') return json(relayCheck);
    if (url.startsWith('/api/pairing/') || url.startsWith('/api/nodes/')) return json({}, 204);
    return json({}, 404);
  });
  vi.stubGlobal('fetch', f);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Devices', () => {
  it('fields and actions are visibly labelled', async () => {
    render(<Devices />);
    await screen.findByLabelText('Node name');
    expect(screen.getByPlaceholderText('Device name')).toBe(screen.getByLabelText('Node name'));
    expect(screen.getByPlaceholderText('relay.example.com')).toBe(screen.getByLabelText('Relay address'));
    expect(screen.getByPlaceholderText('Relay key')).toBe(screen.getByLabelText('Relay key'));
    expect(screen.getByRole('button', { name: 'Add device' })).toHaveTextContent('Add device');
    expect(screen.getByPlaceholderText('Code from the other device')).toBe(screen.getByLabelText('Code'));
    expect(screen.getByRole('button', { name: 'Join' })).toBeDisabled();
  });

  it('shows node name, relay url and members', async () => {
    render(<Devices />);
    expect(await screen.findByLabelText('Node name')).toHaveValue('Home');
    expect(screen.getByLabelText('Relay address')).toHaveValue('ws://r/relay');
    expect(screen.getByLabelText('Relay key')).toHaveValue('');
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
    await userEvent.type(await screen.findByLabelText('Code'), 'ws://r/relay#TOK');
    pending = json({ sas: '111222', peer_name: 'hostid' });
    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    const join = calls('POST', '/api/pairing/join')[0];
    expect(JSON.parse((join[1] as RequestInit).body as string)).toEqual({ code: 'ws://r/relay#TOK', name: 'Home' });
    expect(await screen.findByText('111222', {}, { timeout: 4000 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(calls('POST', '/api/pairing/confirm')).toHaveLength(1);
  });

  it('confirm carries the trust private toggle (default off)', async () => {
    pending = json({ sas: '123456', peer_name: 'Phone' });
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('123456', {}, { timeout: 4000 });
    const toggle = screen.getByRole('checkbox', { name: 'Trust private' });
    expect(toggle).not.toBeChecked();
    await userEvent.click(toggle);
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    const c = calls('POST', '/api/pairing/confirm')[0];
    expect(JSON.parse((c[1] as RequestInit).body as string)).toEqual({ trust_private: true, allow_agents: false });
  });

  it('confirm without toggling sends trust_private false', async () => {
    pending = json({ sas: '123456', peer_name: 'Phone' });
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('123456', {}, { timeout: 4000 });
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    const c = calls('POST', '/api/pairing/confirm')[0];
    expect(JSON.parse((c[1] as RequestInit).body as string)).toEqual({ trust_private: false, allow_agents: false });
  });

  it('joiner does not get the toggle', async () => {
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.type(await screen.findByLabelText('Code'), 'ws://r/relay#TOK');
    pending = json({ sas: '111222', peer_name: 'hostid' });
    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    await screen.findByText('111222', {}, { timeout: 4000 });
    expect(screen.queryByRole('checkbox', { name: 'Trust private' })).toBeNull();
  });

  it('per-member trust toggle, lock icon and remove', async () => {
    render(<Devices />);
    await screen.findByText('Vps');
    // Lock only next to the full-trust member.
    expect(screen.getAllByTestId('trust-lock')).toHaveLength(1);
    const vps = screen.getByRole('checkbox', { name: 'Trust private Vps' });
    const pc = screen.getByRole('checkbox', { name: 'Trust private Pc' });
    expect(vps).not.toBeChecked();
    expect(pc).toBeChecked();
    await userEvent.click(vps);
    const up = calls('PATCH', '/api/nodes/vps/trust')[0];
    expect(JSON.parse((up[1] as RequestInit).body as string)).toEqual({ trust: 'full' });
    await userEvent.click(pc);
    const down = calls('PATCH', '/api/nodes/pc/trust')[0];
    expect(JSON.parse((down[1] as RequestInit).body as string)).toEqual({ trust: 'public' });
    await userEvent.click(screen.getByRole('button', { name: 'Remove Vps' }));
    expect(calls('DELETE', '/api/nodes/vps')).toHaveLength(1);
  });

  it('per-member agents toggle', async () => {
    render(<Devices />);
    await screen.findByText('Vps');
    const vps = screen.getByRole('checkbox', { name: 'Allow agents Vps' });
    const pc = screen.getByRole('checkbox', { name: 'Allow agents Pc' });
    expect(vps).not.toBeChecked();
    expect(pc).toBeChecked();
    await userEvent.click(vps);
    expect(JSON.parse((calls('PATCH', '/api/nodes/vps/agents')[0][1] as RequestInit).body as string)).toEqual({ agents: true });
    await userEvent.click(pc);
    expect(JSON.parse((calls('PATCH', '/api/nodes/pc/agents')[0][1] as RequestInit).body as string)).toEqual({ agents: false });
  });

  it('SAS dialog carries allow_agents for host and joiner', async () => {
    pending = json({ sas: '123456', peer_name: 'Phone' });
    render(<Devices />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('123456', {}, { timeout: 4000 });
    await userEvent.click(screen.getByRole('checkbox', { name: 'Allow agents' }));
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    const c = calls('POST', '/api/pairing/confirm')[0];
    expect(JSON.parse((c[1] as RequestInit).body as string)).toEqual({ trust_private: false, allow_agents: true });
  });

  it('relay address and key are separate fields, saved as one url', async () => {
    settings.relay_url = 'wss://relay.example.com/relay?key=old';
    try {
      render(<Devices />);
      const address = await screen.findByLabelText('Relay address');
      const key = screen.getByLabelText('Relay key');
      expect(address).toHaveValue('wss://relay.example.com/relay');
      expect(key).toHaveValue('old');
      expect(key).toHaveAttribute('type', 'password');
      await userEvent.click(screen.getByRole('button', { name: 'Show key' }));
      expect(key).toHaveAttribute('type', 'text');
      await userEvent.clear(address);
      await userEvent.type(address, 'other.example.com');
      await userEvent.clear(key);
      await userEvent.type(key, 'new');
      await userEvent.tab();
      await waitFor(() => {
        const bodies = calls('PATCH', '/api/settings').map((c) => JSON.parse((c[1] as RequestInit).body as string));
        expect(bodies.at(-1)).toEqual({ relay_url: 'wss://other.example.com/relay?key=new' });
      });
    } finally {
      settings.relay_url = 'ws://r/relay';
    }
  });

  it('shows the live relay status', async () => {
    relayStatus = { configured: true, active: true, online: false, error: 'relay key missing or wrong' };
    render(<Devices />);
    expect(await screen.findByRole('status')).toHaveTextContent('relay key missing or wrong');
  });

  it('check tests the relay as typed and shows the result', async () => {
    relayCheck = { ok: false, error: 'relay key missing or wrong' };
    render(<Devices />);
    const key = await screen.findByLabelText('Relay key');
    await userEvent.type(key, 'typo');
    await userEvent.click(screen.getByRole('button', { name: 'Check relay' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('relay key missing or wrong'));
    const body = JSON.parse((calls('POST', '/api/relay/check')[0][1] as RequestInit).body as string);
    expect(body).toEqual({ relay_url: 'ws://r/relay?key=typo' });
    relayCheck = { ok: true, error: null };
    await userEvent.click(screen.getByRole('button', { name: 'Check relay' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Relay OK'));
  });

  it('invite code can be copied', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    render(<Devices />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add device' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Copy code' }));
    expect(writeText).toHaveBeenCalledWith('ws://r/relay#TOK');
  });
});
