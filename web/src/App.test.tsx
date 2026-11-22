import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from './App';
import { useThreadsStore } from './stores/threads';
import { useNodesStore } from './stores/nodes';
import { useFilesStore } from './stores/files';
import { useChatStore } from './stores/chat';
import type { Thread } from './api/types';

const t = (id: string, title: string): Thread => ({
  id, title, provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
});

function mockWidth(width: number) {
  window.matchMedia = ((q: string) => {
    const max = /max-width:\s*(\d+)/.exec(q);
    return {
      matches: max ? width <= Number(max[1]) : false,
      media: q,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      onchange: null,
      dispatchEvent: () => false,
    } as unknown as MediaQueryList;
  }) as typeof window.matchMedia;
}

beforeEach(() => {
  vi.spyOn(useNodesStore.getState(), 'load').mockResolvedValue(undefined as never);
  useNodesStore.setState({ load: vi.fn().mockResolvedValue(undefined) } as never);
  useThreadsStore.setState({
    threads: [t('a', 'First'), t('b', 'Second')],
    providers: [],
    activeId: 'a',
    menuOpen: false,
    load: vi.fn().mockResolvedValue(undefined),
    loadProviders: vi.fn().mockResolvedValue(undefined),
  } as never);
  useChatStore.setState({ load: vi.fn().mockResolvedValue(undefined) } as never);
  useFilesStore.setState({ load: vi.fn().mockResolvedValue(undefined) } as never);
});

afterEach(() => {
  // @ts-expect-error matchMedia is optional in jsdom
  delete window.matchMedia;
});

describe('App phone layout', () => {
  it('desktop: sidebar and files panel inline, no menu button', () => {
    mockWidth(1024);
    render(<App />);
    expect(screen.getByText('First')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Menu' })).toBeNull();
  });

  it('without matchMedia falls back to desktop', () => {
    render(<App />);
    expect(screen.getByText('First')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Menu' })).toBeNull();
  });

  it('phone: sidebar hidden until menu opens the drawer; close button closes it', async () => {
    mockWidth(375);
    render(<App />);
    expect(screen.queryByText('Second')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    expect(screen.getByText('Second')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Close menu' }));
    expect(screen.queryByText('Second')).toBeNull();
  });

  it('phone: the drawer close button does not cover the new thread button', async () => {
    mockWidth(375);
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    // "+" sits at the sidebar's top right; the close button takes the empty top left.
    const close = screen.getByRole('button', { name: 'Close menu' });
    expect(close.className).toMatch(/\bleft-1\b/);
    expect(close.className).not.toMatch(/\bright-1\b/);
  });

  it('phone: selecting a thread closes the drawer', async () => {
    mockWidth(375);
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    await userEvent.click(screen.getByText('Second'));
    expect(useThreadsStore.getState().activeId).toBe('b');
    expect(screen.queryByText('First')).toBeNull();
  });

  it('phone: files open as a sheet and close again', async () => {
    mockWidth(375);
    render(<App />);
    expect(screen.queryByRole('tab', { name: 'Private' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Files' }));
    expect(screen.getByRole('tab', { name: 'Private' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Close files' }));
    expect(screen.queryByRole('tab', { name: 'Private' })).toBeNull();
  });

  it('phone: files sheet uploads through a file input', async () => {
    mockWidth(375);
    const upload = vi.fn().mockResolvedValue(undefined);
    useFilesStore.setState({ upload } as never);
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Files' }));
    const input = screen.getByLabelText('Upload file') as HTMLInputElement;
    expect(input.type).toBe('file');
    const f = new File(['x'], 'a.txt');
    await userEvent.upload(input, f);
    expect(upload).toHaveBeenCalledWith('public', '', [f]);
  });
});

describe('App orientation', () => {
  it('with no threads the page offers to start one', async () => {
    mockWidth(1024);
    useThreadsStore.setState({ threads: [], activeId: null } as never);
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Start a thread' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('phone: starting a thread opens the drawer with the menu', async () => {
    mockWidth(375);
    useThreadsStore.setState({ threads: [], activeId: null } as never);
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Start a thread' }));
    expect(screen.getByRole('button', { name: 'Close menu' })).toBeInTheDocument();
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('phone: the top bar names the open thread', () => {
    mockWidth(375);
    render(<App />);
    expect(screen.getByRole('banner')).toHaveTextContent('First');
  });

  it('phone: the top bar names a remote node', () => {
    mockWidth(375);
    useNodesStore.setState({
      nodes: [
        { node_id: 'me', name: 'laptop', is_self: true, online: true, trust: null, agents: null },
        { node_id: 'pc', name: 'desk-pc', is_self: false, online: true, trust: 'full', agents: true },
      ],
      selectedId: 'pc',
    } as never);
    render(<App />);
    expect(screen.getByRole('banner')).toHaveTextContent('desk-pc');
    act(() => useNodesStore.setState({ selectedId: null } as never));
    expect(screen.getByRole('banner')).not.toHaveTextContent('desk-pc');
  });
});

describe('App thread switch', () => {
  it('shows only the active thread feed after switching threads', async () => {
    mockWidth(1024);
    const msg = (id: string, thread: string, content: string) => ({
      id, thread_id: thread, role: 'user' as const, content, attachments: [], created_at: 1,
    });
    useChatStore.setState({
      messages: { a: [msg('1', 'a', 'only in a')], b: [msg('2', 'b', 'only in b')] },
    } as never);
    const { rerender } = render(<App />);
    expect(screen.getByText('only in a')).toBeInTheDocument();
    for (const id of ['b', 'a', 'b']) {
      useThreadsStore.setState({ activeId: id } as never);
      rerender(<App />);
    }
    expect(screen.getByText('only in b')).toBeInTheDocument();
    expect(screen.queryByText('only in a')).toBeNull();
    expect(document.querySelectorAll('textarea')).toHaveLength(1);
  });
});
