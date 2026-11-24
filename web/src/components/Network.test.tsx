import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Network } from './Network';

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
let pairing: unknown;
let network: unknown;
const joined = { joined: true, name: 'Home net', relay_host: 'r', devices: 3 };
const alone = { joined: false, name: null, relay_host: null, devices: 1 };
const settings = { ollama_url: '', openrouter_api_key: null, history_char_budget: 1, relay_url: 'ws://r/relay', node_name: 'Home', network_name: null };
const openRelay = async () => {
  await userEvent.click(await screen.findByRole('button', { name: 'Network menu' }));
  await userEvent.click(screen.getByRole('menuitem', { name: 'Relay' }));
};
const calls = (m: string, u: string) =>
  f.mock.calls.filter((c) => c[0] === u && ((c[1] as RequestInit | undefined)?.method ?? 'GET') === m);

beforeEach(() => {
  pending = null;
  relayStatus = { configured: true, active: true, online: true, error: null };
  relayCheck = { ok: true, error: null };
  pairing = { stage: 'idle', invite: null };
  network = joined;
  f = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/network') return json(network);
    if (url === '/api/network/create') return json((network = joined));
    if (url === '/api/network/leave') { network = alone; return new Response(null, { status: 204 }); }
    if (url === '/api/settings' && init?.method === 'PATCH') return json({ ...settings, ...JSON.parse(init.body as string) });
    if (url === '/api/settings') return json(settings);
    if (url === '/api/pairing') return json(pairing);
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

describe('Network: in a network', () => {
  it('shows the network, this device and the members; no joining here', async () => {
    render(<Network />);
    expect(await screen.findByRole('heading', { name: 'Home net' })).toBeInTheDocument();
    expect(screen.getByText('r · 3 devices')).toBeInTheDocument();
    expect(await screen.findByLabelText('Node name')).toHaveValue('Home');
    expect(screen.getByPlaceholderText('Device name')).toBe(screen.getByLabelText('Node name'));
    expect(await screen.findByText('Vps')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add device' })).toHaveTextContent('Add device');
    expect(screen.queryByLabelText('Code from the other device')).toBeNull();
    expect(screen.queryByLabelText('Relay address')).toBeNull();
  });

  it('escape closes the menu without closing Settings', async () => {
    const onDocKey = vi.fn();
    document.addEventListener('keydown', onDocKey);
    try {
      render(<Network />);
      await userEvent.click(await screen.findByRole('button', { name: 'Network menu' }));
      expect(screen.getByRole('menu')).toBeInTheDocument();
      await userEvent.keyboard('{Escape}');
      expect(screen.queryByRole('menu')).toBeNull();
      expect(onDocKey).not.toHaveBeenCalledWith(expect.objectContaining({ key: 'Escape' }));
    } finally {
      document.removeEventListener('keydown', onDocKey);
    }
  });

  it('the menu renames the network', async () => {
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Network menu' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Rename network' }));
    const field = screen.getByLabelText('Network name');
    expect(field).toHaveValue('Home net');
    await userEvent.clear(field);
    await userEvent.type(field, 'Work{Enter}');
    const bodies = calls('PATCH', '/api/settings').map((c) => JSON.parse((c[1] as RequestInit).body as string));
    expect(bodies).toContainEqual({ network_name: 'Work' });
  });

  it('leave asks first, then shows the choice again', async () => {
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Network menu' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Leave network' }));
    expect(calls('POST', '/api/network/leave')).toHaveLength(0);
    await userEvent.click(screen.getByRole('button', { name: 'Leave' }));
    expect(calls('POST', '/api/network/leave')).toHaveLength(1);
    expect(await screen.findByRole('button', { name: 'Join a network' })).toBeInTheDocument();
  });

  it('saves the node name on blur', async () => {
    render(<Network />);
    const name = await screen.findByLabelText('Node name');
    await userEvent.clear(name);
    await userEvent.type(name, 'Box');
    await userEvent.tab();
    const c = f.mock.calls.find((x) => (x[1] as RequestInit | undefined)?.method === 'PATCH');
    expect(JSON.parse(((c as unknown[])[1] as RequestInit).body as string)).toEqual({ node_name: 'Box' });
  });

  it('add device shows code with countdown, then SAS and confirm', async () => {
    render(<Network />);
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
    render(<Network />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    expect(await screen.findByText('654321', {}, { timeout: 4000 })).toBeInTheDocument();
    pending = null;
    await userEvent.click(screen.getByRole('button', { name: 'Reject' }));
    expect(calls('POST', '/api/pairing/reject')).toHaveLength(1);
    await waitFor(() => expect(screen.queryByText('654321')).toBeNull());
  });


  it('confirm carries the trust private toggle (default off)', async () => {
    pending = json({ sas: '123456', peer_name: 'Phone' });
    render(<Network />);
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
    render(<Network />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('123456', {}, { timeout: 4000 });
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    const c = calls('POST', '/api/pairing/confirm')[0];
    expect(JSON.parse((c[1] as RequestInit).body as string)).toEqual({ trust_private: false, allow_agents: false });
  });


  it('per-member trust toggle, lock icon and remove', async () => {
    render(<Network />);
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
    render(<Network />);
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
    render(<Network />);
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
      render(<Network />);
      await openRelay();
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
    render(<Network />);
    await openRelay();
    expect(await screen.findByRole('status')).toHaveTextContent('relay key missing or wrong');
  });

  it('check tests the relay as typed and shows the result', async () => {
    relayCheck = { ok: false, error: 'relay key missing or wrong' };
    render(<Network />);
    await openRelay();
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
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add device' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Copy code' }));
    expect(writeText).toHaveBeenCalledWith('ws://r/relay#TOK');
  });

  it('each device names its permissions in words', async () => {
    render(<Network />);
    await screen.findByText('Vps');
    expect(screen.getAllByText('Private files')).toHaveLength(2);
    expect(screen.getAllByText('Run agents here')).toHaveLength(2);
    expect(screen.getByText('Syncs the private space with Vps')).toBeInTheDocument();
    expect(screen.getByText('Vps can start Claude and Antigravity on Home')).toBeInTheDocument();
  });

  it('when adding a device the host shows whom the new device will meet and how', async () => {
    pending = json({ sas: '123456', peer_name: 'Phone' });
    render(<Network />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('123456', {}, { timeout: 4000 });
    const meet = screen.getByRole('list', { name: 'Phone will also meet' });
    // Without private trust the new device marks every member public.
    expect(meet).toHaveTextContent('Vpspublic files only');
    expect(meet).toHaveTextContent('Pcpublic files only');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Trust private' }));
    // With it, the new device takes this device's view: Pc is trusted, Vps is not.
    expect(meet).toHaveTextContent('Pcprivate files');
    expect(meet).toHaveTextContent('Vpspublic files only');
  });

  it('reopening Settings during an invite shows the same code again', async () => {
    console.log('[Step 1/2] the node is inviting with a code from before');
    pairing = { stage: 'inviting', invite: { code: 'ws://r/relay#OLD', expires_at: Math.floor(Date.now() / 1000) + 200 } };
    render(<Network />);
    console.log('[Step 2/2] the code card is back, no new invite was asked for');
    expect(await screen.findByText('ws://r/relay#OLD')).toBeInTheDocument();
    expect(screen.getByText('Waiting for the other device')).toBeInTheDocument();
    expect(calls('POST', '/api/pairing/invite')).toHaveLength(0);
  });

  it('reopening Settings while joining shows the connecting card', async () => {
    pairing = { stage: 'joining', invite: null };
    render(<Network />);
    expect(await screen.findByText('Connecting to the other device')).toBeInTheDocument();
  });

  it('cancel on the code card ends the invite and brings back Add device', async () => {
    render(<Network />);
    await screen.findByLabelText('Node name');
    await userEvent.click(screen.getByRole('button', { name: 'Add device' }));
    await screen.findByText('ws://r/relay#TOK');
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(calls('POST', '/api/pairing/cancel')).toHaveLength(1);
    expect(screen.queryByText('ws://r/relay#TOK')).toBeNull();
    expect(screen.getByRole('button', { name: 'Add device' })).toBeInTheDocument();
  });

  it('cancel while connecting ends the join', async () => {
    pairing = { stage: 'joining', invite: null };
    render(<Network />);
    await screen.findByText('Connecting to the other device');
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(calls('POST', '/api/pairing/cancel')).toHaveLength(1);
    expect(screen.queryByText('Connecting to the other device')).toBeNull();
  });

  it('only the code card and the connecting card offer Cancel', async () => {
    render(<Network />);
    await screen.findByLabelText('Node name');
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
  });
});

describe('Network: not in a network', () => {
  const code = 'ws://r/relay#AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';
  beforeEach(() => {
    network = alone;
  });

  it('offers Join first and Create second, nothing else', async () => {
    render(<Network />);
    const join = await screen.findByRole('button', { name: 'Join a network' });
    const create = screen.getByRole('button', { name: 'Create a network' });
    expect(join.compareDocumentPosition(create) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Add device' })).toBeNull();
    expect(screen.queryByLabelText('Node name')).toBeNull();
  });

  it('join sends the code, connects, then shows the numbers without the private toggle', async () => {
    console.log('[Step 1/3] open Join and paste a device code');
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Join a network' }));
    const field = screen.getByLabelText('Code from the other device');
    expect(screen.getByRole('button', { name: 'Join' })).toBeDisabled();
    await userEvent.type(field, code);
    expect(screen.getByText('Joins the network on r')).toBeInTheDocument();
    console.log('[Step 2/3] join');
    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    const body = JSON.parse((calls('POST', '/api/pairing/join')[0][1] as RequestInit).body as string);
    expect(body).toEqual({ code, name: 'Home' });
    expect(await screen.findByText('Connecting to the other device')).toBeInTheDocument();
    console.log('[Step 3/3] the numbers come, confirm');
    pending = json({ sas: '111222', peer_name: 'hostid' });
    expect(await screen.findByText('111222', {}, { timeout: 4000 })).toBeInTheDocument();
    expect(screen.queryByRole('checkbox', { name: 'Trust private' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(calls('POST', '/api/pairing/confirm')).toHaveLength(1);
  });

  it('a relay code pasted into Join offers to create instead', async () => {
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Join a network' }));
    await userEvent.type(screen.getByLabelText('Code from the other device'), 'relay.example.com?key=k');
    expect(screen.getByRole('button', { name: 'Join' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'Create a network with it' }));
    expect(screen.getByLabelText('Relay code')).toHaveValue('relay.example.com?key=k');
  });

  it('create checks the relay, then starts the network', async () => {
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Create a network' }));
    await userEvent.type(screen.getByLabelText('Relay code'), 'relay.example.com?key=k');
    expect(screen.getByLabelText('Network name')).toHaveAttribute('placeholder', 'relay.example.com');
    await userEvent.type(screen.getByLabelText('Network name'), 'Home');
    await userEvent.click(screen.getByRole('button', { name: 'Create network' }));
    const check = JSON.parse((calls('POST', '/api/relay/check')[0][1] as RequestInit).body as string);
    expect(check).toEqual({ relay_url: 'wss://relay.example.com/relay?key=k' });
    const body = JSON.parse((calls('POST', '/api/network/create')[0][1] as RequestInit).body as string);
    expect(body).toEqual({ relay_url: 'wss://relay.example.com/relay?key=k', name: 'Home' });
    expect(await screen.findByRole('button', { name: 'Add device' })).toBeInTheDocument();
  });

  it('create stops when the relay does not answer', async () => {
    relayCheck = { ok: false, error: 'relay key missing or wrong' };
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Create a network' }));
    await userEvent.type(screen.getByLabelText('Relay code'), 'relay.example.com?key=bad');
    await userEvent.click(screen.getByRole('button', { name: 'Create network' }));
    expect(await screen.findByText('relay key missing or wrong')).toBeInTheDocument();
    expect(calls('POST', '/api/network/create')).toHaveLength(0);
  });

  it('back returns to the choice', async () => {
    render(<Network />);
    await userEvent.click(await screen.findByRole('button', { name: 'Create a network' }));
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(screen.getByRole('button', { name: 'Join a network' })).toBeInTheDocument();
  });

  it('reopening Settings while joining shows the connecting card', async () => {
    pairing = { stage: 'joining', invite: null };
    render(<Network />);
    expect(await screen.findByText('Connecting to the other device')).toBeInTheDocument();
  });
});
