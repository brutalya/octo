import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sidebar } from './Sidebar';
import { useThreadsStore } from '../stores/threads';
import { useChatStore } from '../stores/chat';
import type { ProviderStatus, Thread } from '../api/types';

const providers: ProviderStatus[] = [
  { kind: 'claude', installed: true, version: '1', is_local: false },
  { kind: 'antigravity', installed: true, version: '1', is_local: true },
  { kind: 'ollama', installed: true, version: null, is_local: true },
  { kind: 'openrouter', installed: true, version: null, is_local: false },
];

const t = (id: string, title: string): Thread => ({
  id, title, provider: 'claude', space: 'public', approval: 'confirm', model: null, cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
});

beforeEach(() => {
  useThreadsStore.setState({ threads: [t('a', 'First'), t('b', 'Second')], providers, activeId: 'a', menuOpen: false });
});

describe('Sidebar', () => {
  it('lists threads with a muted provider label', () => {
    render(<Sidebar />);
    expect(screen.getByText('First')).toBeInTheDocument();
    expect(screen.getAllByText('claude').length).toBe(2);
  });

  it('an untitled thread reads as Untitled, not a blank row', () => {
    useThreadsStore.setState({ threads: [t('a', '')] });
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'Untitled' })).toHaveTextContent('Untitled');
  });

  it('selects and deletes threads', async () => {
    const select = vi.fn();
    const remove = vi.fn(async () => {});
    useThreadsStore.setState({ select, remove });
    render(<Sidebar />);
    await userEvent.click(screen.getByText('Second'));
    expect(select).toHaveBeenCalledWith('b');
    const del = screen.getAllByRole('button', { name: 'Delete thread' })[0];
    await userEvent.click(del);
    expect(remove).not.toHaveBeenCalled();
    await userEvent.click(del);
    expect(remove).toHaveBeenCalledWith('a');
  });

  it('delete confirmation expires after 3 seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const remove = vi.fn(async () => {});
    useThreadsStore.setState({ remove });
    render(<Sidebar />);
    const del = screen.getAllByRole('button', { name: 'Delete thread' })[0];
    await userEvent.click(del);
    await act(async () => {
      vi.advanceTimersByTime(3100);
    });
    await userEvent.click(del);
    expect(remove).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('shows the store error at the bottom', () => {
    useThreadsStore.setState({ error: 'boom' });
    render(<Sidebar />);
    expect(screen.getByText('boom')).toBeInTheDocument();
    useThreadsStore.setState({ error: null });
  });

  it('groups threads into Today and Earlier', () => {
    const now = Date.now();
    useThreadsStore.setState({
      threads: [{ ...t('a', 'Fresh'), updated_at: now }, { ...t('b', 'Old'), updated_at: now - 3 * 86_400_000 }],
    });
    render(<Sidebar />);
    const today = screen.getByRole('list', { name: 'Today' });
    const earlier = screen.getByRole('list', { name: 'Earlier' });
    expect(within(today).getByText('Fresh')).toBeInTheDocument();
    expect(within(earlier).getByText('Old')).toBeInTheDocument();
  });

  it('search narrows the thread list by title', async () => {
    render(<Sidebar />);
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search threads' }), 'sec');
    expect(screen.queryByText('First')).toBeNull();
    expect(screen.getByText('Second')).toBeInTheDocument();
  });

  it('marks private threads with a lock and running threads as running', () => {
    useThreadsStore.setState({ threads: [{ ...t('a', 'Notes'), space: 'private' }, t('b', 'Busy')] });
    useChatStore.setState({ streaming: { b: { text: '', tool: null } } });
    render(<Sidebar />);
    expect(screen.getByRole('img', { name: 'Private' })).toBeInTheDocument();
    expect(screen.getByText('claude · running')).toBeInTheDocument();
    useChatStore.setState({ streaming: {} });
  });

  it('new thread and settings are labelled buttons', () => {
    render(<Sidebar />);
    expect(screen.getByRole('button', { name: 'New thread' })).toHaveTextContent('New thread');
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveTextContent('Settings');
  });

  it('a close button is shown only when the sidebar is a drawer', async () => {
    const onClose = vi.fn();
    const { rerender } = render(<Sidebar />);
    expect(screen.queryByRole('button', { name: 'Close menu' })).toBeNull();
    rerender(<Sidebar onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: 'Close menu' }));
    expect(onClose).toHaveBeenCalled();
  });
});

describe('Sidebar http providers', () => {
  const models = [
    { id: 'llama3:8b', name: 'Llama 3 8B' },
    { id: 'qwen2:7b', name: 'Qwen 2 7B' },
  ];
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(models), { status: 200 })));
  });
  afterEach(() => vi.unstubAllGlobals());

  it('thread label shows provider and short model name', () => {
    useThreadsStore.setState({
      threads: [
        { ...t('a', 'First'), provider: 'ollama', model: 'llama3:latest' },
        { ...t('b', 'Second'), provider: 'openrouter', model: 'anthropic/claude-3.5-sonnet' },
      ],
    });
    render(<Sidebar />);
    expect(screen.getByText('ollama · llama3')).toBeInTheDocument();
    expect(screen.getByText('openrouter · claude-3.5-sonnet')).toBeInTheDocument();
  });

  it('gear opens the settings modal', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });
});

describe('Sidebar new thread', () => {
  it('the New thread button opens the new thread dialog', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    expect(useThreadsStore.getState().menuOpen).toBe(true);
  });
});
