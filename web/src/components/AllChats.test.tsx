import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AllChats } from './AllChats';
import { useThreadsStore } from '../stores/threads';
import { useNodesStore } from '../stores/nodes';
import { useDevicesStore } from '../stores/devices';
import { usePinsStore } from '../stores/pins';
import type { NodeInfo, Thread } from '../api/types';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
// Noon today, so "an hour ago" is still today and "a day ago" is yesterday.
const noon = new Date();
noon.setHours(12, 0, 0, 0);
const at = noon.getTime();

const t = (id: string, title: string, ago: number): Thread => ({
  id, title, provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: '/home/u/octo', origin: 'octo', created_at: 1, updated_at: at - ago,
});
const node = (node_id: string, name: string, is_self = false, online = true): NodeInfo => ({
  node_id, name, is_self, online, trust: is_self ? null : 'full', agents: is_self ? null : true,
});
const session = (id: string, title: string, ago: number, imported: string | null = null) => ({
  external_id: id, title, cwd: '/home/u/octo', updated_at: at - ago, messages: 3, imported_thread_id: imported,
});

const real = { nodeSelect: useNodesStore.getState().select, select: useThreadsStore.getState().select };
let f: ReturnType<typeof vi.fn>;
const importSession = vi.fn(async () => {});

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true, now: at + HOUR });
  localStorage.clear();
  usePinsStore.setState({ pins: [] });
  useNodesStore.setState({
    nodes: [node('me', 'phone', true), node('pc', 'pc'), node('lin', 'linux'), node('lap', 'laptop', false, false)],
    selectedId: 'pc',
    select: vi.fn(),
  });
  useThreadsStore.setState({
    threads: [t('p1', 'Refactor the relay', HOUR), t('p2', 'Relay deploy notes', DAY)],
    activeId: 'p1',
    select: real.select,
    importSession,
  });
  useDevicesStore.setState({
    lists: { lin: [t('l1', 'Relay on a VPS', 3 * DAY)], me: [], lap: [t('x', 'Relay offline', HOUR)] },
    load: async () => {},
  });
  f = vi.fn(async (url: string) => {
    const u = new URL(url, 'http://x');
    const q = u.searchParams.get('q') ?? '';
    let list: unknown[] = [];
    if (u.pathname === '/api/nodes/pc/import/claude/sessions') {
      list = [session('s1', 'Relay secret rotation', 2 * HOUR), session('s2', 'Already here', HOUR, 'p1')];
    } else if (u.pathname === '/api/nodes/pc/import/antigravity/sessions') {
      list = [session('a1', 'Relay websocket bug', DAY + HOUR)];
    } else if (u.pathname === '/api/nodes/lin/import/claude/sessions') {
      return new Response('{"error":"forbidden"}', { status: 403 });
    }
    return new Response(JSON.stringify(list.filter((s) => (s as { title: string }).title.toLowerCase().includes(q))));
  });
  vi.stubGlobal('fetch', f);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  useNodesStore.setState({ select: real.nodeSelect });
});

const titles = (el: HTMLElement) => within(el).getAllByRole('button', { name: /^Open / }).map((b) => b.getAttribute('aria-label'));

describe('AllChats', () => {
  it('merges chats and outside sessions of every reachable device, newest first, by day', async () => {
    console.log('[Step 1/2] open All chats');
    render(<AllChats device={null} onBack={() => {}} />);
    console.log('[Step 2/2] Today / Yesterday / This week; imported sessions and offline devices left out');
    const today = await screen.findByRole('list', { name: 'Today' });
    await waitFor(() => expect(titles(today)).toContain('Open Relay secret rotation'));
    expect(titles(today)).toEqual(['Open Refactor the relay', 'Open Relay secret rotation']);
    expect(titles(screen.getByRole('list', { name: 'Yesterday' }))).toEqual([
      'Open Relay deploy notes', 'Open Relay websocket bug',
    ]);
    expect(titles(screen.getByRole('list', { name: 'This week' }))).toEqual(['Open Relay on a VPS']);
    expect(screen.queryByText('Already here')).toBeNull();
    expect(screen.queryByText('Relay offline')).toBeNull();
    expect(screen.getByText('from Claude Code')).toBeInTheDocument();
    expect(screen.getByText('from Antigravity')).toBeInTheDocument();
  });

  it('a device chip narrows the list; opening from a device starts on its chip', async () => {
    render(<AllChats device="lin" onBack={() => {}} />);
    expect(screen.getByRole('button', { name: 'linux' })).toHaveAttribute('aria-pressed', 'true');
    expect(await screen.findByText('Relay on a VPS')).toBeInTheDocument();
    expect(screen.queryByText('Refactor the relay')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'All devices' }));
    expect(await screen.findByText('Refactor the relay')).toBeInTheDocument();
  });

  it('search narrows chats and asks devices for matching sessions', async () => {
    render(<AllChats device={null} onBack={() => {}} />);
    await screen.findByText('Relay secret rotation');
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search all chats' }), 'websocket');
    await waitFor(() => expect(screen.queryByText('Relay secret rotation')).toBeNull());
    expect(screen.getByText('Relay websocket bug')).toBeInTheDocument();
    expect(screen.queryByText('Refactor the relay')).toBeNull();
    expect(f.mock.calls.some((c) => String(c[0]).includes('q=websocket'))).toBe(true);
  });

  it('opening a chat on another device switches there', async () => {
    const onNavigate = vi.fn();
    render(<AllChats device={null} onBack={() => {}} onNavigate={onNavigate} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Open Relay on a VPS' }));
    expect(useNodesStore.getState().select).toHaveBeenCalledWith('lin');
    expect(useThreadsStore.getState().activeId).toBe('l1');
    expect(onNavigate).toHaveBeenCalled();
  });

  it('opening an outside session continues it in Octo on its device', async () => {
    useNodesStore.setState({ selectedId: 'lin' });
    render(<AllChats device={null} onBack={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Open Relay secret rotation' }));
    expect(useNodesStore.getState().select).toHaveBeenCalledWith('pc');
    expect(importSession).toHaveBeenCalledWith('claude', 's1');
  });

  it('the Pinned chip shows pinned chats only', async () => {
    usePinsStore.setState({ pins: ['lin/l1'] });
    render(<AllChats device={null} onBack={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Pinned' }));
    await waitFor(() => expect(screen.queryByText('Refactor the relay')).toBeNull());
    expect(screen.getByText('Relay on a VPS')).toBeInTheDocument();
    expect(screen.queryByText('Relay secret rotation')).toBeNull();
  });

  it('back closes it', async () => {
    const onBack = vi.fn();
    render(<AllChats device={null} onBack={onBack} />);
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(onBack).toHaveBeenCalled();
  });
});
