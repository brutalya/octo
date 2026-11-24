import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sidebar } from './Sidebar';
import { useThreadsStore } from '../stores/threads';
import { useChatStore } from '../stores/chat';
import { useNodesStore } from '../stores/nodes';
import { useDevicesStore } from '../stores/devices';
import { usePinsStore } from '../stores/pins';
import type { NodeInfo, ProviderStatus, Thread } from '../api/types';

const providers: ProviderStatus[] = [
  { kind: 'claude', installed: true, version: '1', is_local: false },
  { kind: 'ollama', installed: true, version: null, is_local: true },
  { kind: 'openrouter', installed: true, version: null, is_local: false },
];

const now = Date.now();
const t = (id: string, title: string, ago = 0): Thread => ({
  id, title, provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: now - ago,
});
const node = (node_id: string, name: string, is_self = false, online = true): NodeInfo => ({
  node_id, name, is_self, online, trust: is_self ? null : 'full', agents: is_self ? null : true,
});
const network = { joined: true, name: 'Home', relay_host: 'r', devices: 3 };

/** One device, nothing else: the drawer is a plain list. */
function single(threads: Thread[]) {
  useNodesStore.setState({ nodes: [node('me', 'phone', true)], selectedId: null });
  useThreadsStore.setState({ threads, activeId: threads[0]?.id ?? null });
}

/** phone (this device), pc (open), linux, laptop offline. */
function network3() {
  useNodesStore.setState({
    nodes: [node('me', 'phone', true), node('pc', 'pc'), node('lin', 'linux'), node('lap', 'laptop', false, false)],
    selectedId: 'pc',
  });
  useThreadsStore.setState({
    threads: [t('p1', 'Relay', 1000), t('p2', 'Taxes', 2000), t('p3', 'Notes', 3000), t('p4', 'Old one', 4000)],
    activeId: 'p1',
  });
  useDevicesStore.setState({
    lists: { lin: [t('l1', 'Server backups', 500)], me: [], lap: [t('x1', 'Hidden', 1)] },
    access: { pc: { agents: true, private_files: true }, lin: { agents: true, private_files: true }, lap: { agents: true, private_files: true } },
  });
}

// Tests replace store actions with spies; put the real ones back before each test.
const real = {
  select: useThreadsStore.getState().select,
  remove: useThreadsStore.getState().remove,
  nodeSelect: useNodesStore.getState().select,
};
let f: ReturnType<typeof vi.fn>;
beforeEach(() => {
  useThreadsStore.setState({ select: real.select, remove: real.remove });
  useNodesStore.setState({ select: real.nodeSelect });
  localStorage.clear();
  usePinsStore.setState({ pins: [] });
  useDevicesStore.setState({ lists: {}, access: {}, load: async () => {} });
  useThreadsStore.setState({ providers, menuOpen: false, error: null });
  f = vi.fn(async (url: string) =>
    new Response(JSON.stringify(url === '/api/network' ? network : []), { status: 200 }),
  );
  vi.stubGlobal('fetch', f);
  single([t('a', 'First'), t('b', 'Second')]);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('Sidebar on one device', () => {
  it('lists chats with provider and age, no device headers', () => {
    render(<Sidebar />);
    expect(screen.getByText('First')).toBeInTheDocument();
    expect(screen.getAllByText('claude · now')).toHaveLength(2);
    expect(screen.queryByRole('button', { name: /^phone/ })).toBeNull();
  });

  it('an untitled chat reads as Untitled, not a blank row', () => {
    single([t('a', '')]);
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'Untitled' })).toHaveTextContent('Untitled');
  });

  it('selects a chat', async () => {
    const select = vi.fn();
    useThreadsStore.setState({ select });
    render(<Sidebar />);
    await userEvent.click(screen.getByText('Second'));
    expect(select).toHaveBeenCalledWith('b');
  });

  it('delete is in the chat menu and needs a second tap', async () => {
    const remove = vi.fn(async () => {});
    useThreadsStore.setState({ remove });
    render(<Sidebar />);
    await userEvent.click(screen.getAllByRole('button', { name: 'Chat actions' })[0]);
    await userEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(remove).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('menuitem', { name: 'Confirm delete' }));
    expect(remove).toHaveBeenCalledWith('a');
  });

  it('delete confirmation expires after 3 seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const remove = vi.fn(async () => {});
    useThreadsStore.setState({ remove });
    render(<Sidebar />);
    await userEvent.click(screen.getAllByRole('button', { name: 'Chat actions' })[0]);
    await userEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    await act(async () => {
      vi.advanceTimersByTime(3100);
    });
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toBeInTheDocument();
    expect(remove).not.toHaveBeenCalled();
  });

  it('shows the store error at the bottom', () => {
    useThreadsStore.setState({ error: 'boom' });
    render(<Sidebar />);
    expect(screen.getByText('boom')).toBeInTheDocument();
  });

  it('marks private chats with a lock and running chats as running', () => {
    single([{ ...t('a', 'Notes'), space: 'private' }, t('b', 'Busy')]);
    useChatStore.setState({ streaming: { b: { text: '', tool: null } } });
    render(<Sidebar />);
    expect(screen.getByRole('img', { name: 'Private' })).toBeInTheDocument();
    expect(screen.getByText('claude · running')).toBeInTheDocument();
    useChatStore.setState({ streaming: {} });
  });

  it('new chat and settings are labelled buttons; new chat opens the dialog', async () => {
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveTextContent('Settings');
    await userEvent.click(screen.getByRole('button', { name: 'New chat' }));
    expect(useThreadsStore.getState().menuOpen).toBe(true);
  });

  it('a close button is shown only when the sidebar is a drawer', async () => {
    const onClose = vi.fn();
    const { rerender } = render(<Sidebar />);
    expect(screen.queryByRole('button', { name: 'Close menu' })).toBeNull();
    rerender(<Sidebar onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: 'Close menu' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('chat label shows provider and short model name', () => {
    single([
      { ...t('a', 'First'), provider: 'ollama', model: 'llama3:latest' },
      { ...t('b', 'Second'), provider: 'openrouter', model: 'anthropic/claude-3.5-sonnet' },
    ]);
    render(<Sidebar />);
    expect(screen.getByText('ollama · llama3 · now')).toBeInTheDocument();
    expect(screen.getByText('openrouter · claude-3.5-sonnet · now')).toBeInTheDocument();
  });

  it('settings opens the settings page', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });
});

describe('Sidebar in a network', () => {
  beforeEach(network3);

  it('names the network and groups chats by device, offline last', async () => {
    render(<Sidebar />);
    expect(await screen.findByText('Home · 3 devices')).toBeInTheDocument();
    const headers = screen.getAllByRole('button', { name: /^(phone|pc|linux|laptop)/ }).map((b) => b.getAttribute('aria-label'));
    // This device has no chats here, so it is not listed.
    expect(headers).toEqual(['pc, online', 'linux, online', 'laptop, offline']);
  });

  it('devices that do not let this one run agents are not listed; this device shows with chats', () => {
    useDevicesStore.setState((s) => ({
      lists: { ...s.lists, me: [t('m1', 'Phone notes', 100)] },
      access: { ...s.access, lin: { agents: false, private_files: true } },
    }));
    render(<Sidebar />);
    expect(screen.queryByRole('button', { name: 'linux, online' })).toBeNull();
    expect(screen.queryByText('Server backups')).toBeNull();
    expect(screen.getByRole('button', { name: 'phone, this device' })).toBeInTheDocument();
  });

  it('shows the 3 most recent chats per device; All chats on pc opens the list on pc', async () => {
    render(<Sidebar />);
    const pc = screen.getByRole('list', { name: 'Chats on pc' });
    expect(within(pc).getAllByRole('button', { name: /^(Relay|Taxes|Notes|Old one)$/ })).toHaveLength(3);
    await userEvent.click(screen.getByRole('button', { name: 'All chats on pc · 4' }));
    expect(screen.getByRole('heading', { name: 'All chats' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'pc' })).toHaveAttribute('aria-pressed', 'true');
    expect(await screen.findByRole('button', { name: 'Open Old one' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(screen.getByRole('list', { name: 'Chats on pc' })).toBeInTheDocument();
  });

  it('an offline device shows no chats', () => {
    render(<Sidebar />);
    expect(screen.queryByText('Hidden')).toBeNull();
    expect(screen.queryByRole('list', { name: 'Chats on laptop' })).toBeNull();
  });

  it('opening a chat on another device switches to that device first', async () => {
    const select = vi.fn();
    useNodesStore.setState({ select });
    const onNavigate = vi.fn();
    render(<Sidebar onNavigate={onNavigate} />);
    await userEvent.click(screen.getByText('Server backups'));
    expect(select).toHaveBeenCalledWith('lin');
    expect(useThreadsStore.getState().activeId).toBe('l1');
    expect(onNavigate).toHaveBeenCalled();
  });

  it('a device group collapses and stays collapsed', async () => {
    const { unmount } = render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'linux, online' }));
    expect(screen.queryByText('Server backups')).toBeNull();
    unmount();
    render(<Sidebar />);
    expect(screen.queryByText('Server backups')).toBeNull();
  });

  it('pin puts a chat in Pinned with its device; unpin takes it out', async () => {
    console.log('[Step 1/3] pin a chat on linux');
    render(<Sidebar />);
    const row = screen.getByRole('button', { name: 'Server backups' }).closest('li') as HTMLElement;
    await userEvent.click(within(row).getByRole('button', { name: 'Chat actions' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Pin' }));
    console.log('[Step 2/3] it is listed under Pinned, saved for next time');
    const pinned = screen.getByRole('list', { name: 'Pinned' });
    expect(within(pinned).getByText('Server backups')).toBeInTheDocument();
    expect(within(pinned).getByText('linux · claude')).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem('octo.pins') ?? '[]')).toEqual(['lin/l1']);
    console.log('[Step 3/3] unpin from the pinned row');
    await userEvent.click(within(pinned).getByRole('button', { name: 'Chat actions' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Unpin' }));
    expect(screen.queryByRole('list', { name: 'Pinned' })).toBeNull();
  });

  it('deleting a chat on another device goes to that device', async () => {
    render(<Sidebar />);
    const row = screen.getByRole('button', { name: 'Server backups' }).closest('li') as HTMLElement;
    await userEvent.click(within(row).getByRole('button', { name: 'Chat actions' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Confirm delete' }));
    expect(f.mock.calls.some((c) => c[0] === '/api/nodes/lin/threads/l1' && (c[1] as RequestInit)?.method === 'DELETE')).toBe(true);
    expect(screen.queryByText('Server backups')).toBeNull();
  });

  it('search opens All chats over every device', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'Search all chats' }));
    expect(screen.getByRole('button', { name: 'All devices' })).toHaveAttribute('aria-pressed', 'true');
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search all chats' }), 'backup');
    expect(await screen.findByRole('button', { name: 'Open Server backups' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open Relay' })).toBeNull();
  });
});
