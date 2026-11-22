import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, within, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sidebar } from './Sidebar';
import { useThreadsStore } from '../stores/threads';
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
  useThreadsStore.setState({ threads: [t('a', 'First'), t('b', 'Second')], providers, activeId: 'a' });
});

describe('Sidebar', () => {
  it('lists threads with a muted provider label', () => {
    render(<Sidebar />);
    expect(screen.getByText('First')).toBeInTheDocument();
    expect(screen.getAllByText('claude').length).toBe(2);
  });

  it('dropdown lists only installed providers', async () => {
    useThreadsStore.setState({ providers: [providers[0]] });
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    const menu = screen.getByRole('menu');
    expect(within(menu).getByText('claude')).toBeInTheDocument();
    expect(within(menu).queryByText('antigravity')).toBeNull();
  });

  it('private is disabled for non-local providers only', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    expect(screen.getByRole('button', { name: 'claude private' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'claude public' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'antigravity private' })).toBeEnabled();
  });

  it('choosing a space creates a thread and closes the menu', async () => {
    const create = vi.fn(async () => {});
    useThreadsStore.setState({ create });
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    await userEvent.click(screen.getByRole('button', { name: 'antigravity private' }));
    expect(create).toHaveBeenCalledWith('antigravity', 'private', 'confirm');
    expect(screen.queryByRole('menu')).toBeNull();
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

  it('approval toggle in the dropdown defaults to confirm and is passed on create', async () => {
    const create = vi.fn(async () => {});
    useThreadsStore.setState({ create });
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    const toggle = screen.getByRole('button', { name: 'Approval: confirm' });
    await userEvent.click(toggle);
    expect(screen.getByRole('button', { name: 'Approval: auto' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'claude public' }));
    expect(create).toHaveBeenCalledWith('claude', 'public', 'auto');
  });

  it('shows a muted approval icon per thread and switches it on click', async () => {
    const setApproval = vi.fn(async () => {});
    useThreadsStore.setState({
      threads: [t('a', 'First'), { ...t('b', 'Second'), approval: 'auto' }],
      setApproval,
    });
    render(<Sidebar />);
    const confirm = screen.getByRole('button', { name: 'Thread approval: confirm' });
    const auto = screen.getByRole('button', { name: 'Thread approval: auto' });
    await userEvent.click(confirm);
    expect(setApproval).toHaveBeenCalledWith('a', 'auto');
    await userEvent.click(auto);
    expect(setApproval).toHaveBeenCalledWith('b', 'confirm');
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

  it('private option exists only for local providers', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    expect(screen.getByRole('button', { name: 'ollama private' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'openrouter private' })).toBeDisabled();
  });

  it('shows a searchable model submenu and creates with the chosen model', async () => {
    const create = vi.fn(async () => {});
    useThreadsStore.setState({ create });
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    await userEvent.click(screen.getByRole('button', { name: 'ollama private' }));
    expect(create).not.toHaveBeenCalled();
    expect(await screen.findByRole('option', { name: /Llama 3 8B/ })).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox'), 'qwen');
    expect(screen.queryByRole('option', { name: /Llama/ })).toBeNull();
    await userEvent.click(screen.getByRole('option', { name: /Qwen 2 7B/ }));
    expect(create).toHaveBeenCalledWith('ollama', 'private', 'confirm', 'qwen2:7b');
    expect(screen.queryByRole('menu')).toBeNull();
  });

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

describe('Sidebar session picker', () => {
  const sessions = [
    { external_id: 's1', title: 'Fix the parser', cwd: '/srv/apps/demo', updated_at: Date.now() - 3 * 3_600_000, messages: 4, imported_thread_id: null },
    { external_id: 's2', title: 'Write docs', cwd: 'C:\\work\\site', updated_at: Date.now() - 2 * 86_400_000, messages: 2, imported_thread_id: null },
  ];
  let fetchMock: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    fetchMock = vi.fn(async (url: string) => {
      const q = new URL(url, 'http://x').searchParams.get('q') ?? '';
      const rows = sessions.filter((x) => x.title.toLowerCase().includes(q.toLowerCase()));
      return new Response(JSON.stringify(rows), { status: 200 });
    });
    vi.stubGlobal('fetch', fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it('only CLI providers offer sessions', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    expect(screen.getByRole('button', { name: 'claude sessions' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'antigravity sessions' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'ollama sessions' })).toBeNull();
  });

  it('lists sessions with title, short cwd and relative time', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    await userEvent.click(screen.getByRole('button', { name: 'claude sessions' }));
    expect(await screen.findByRole('option', { name: /Fix the parser/ })).toBeInTheDocument();
    expect(screen.getByText('apps/demo')).toBeInTheDocument();
    expect(screen.getByText('work/site')).toBeInTheDocument();
    expect(screen.getByText('3h')).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][0]).toBe('/api/import/claude/sessions?q=');
  });

  it('search asks the server and narrows the list', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    await userEvent.click(screen.getByRole('button', { name: 'claude sessions' }));
    await screen.findByRole('option', { name: /Write docs/ });
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search sessions' }), 'docs');
    await waitFor(() => expect(screen.queryByRole('option', { name: /Fix the parser/ })).toBeNull());
    expect(screen.getByRole('option', { name: /Write docs/ })).toBeInTheDocument();
  });

  it('clicking a session imports it and closes the menu', async () => {
    const importSession = vi.fn(async () => {});
    useThreadsStore.setState({ importSession });
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    await userEvent.click(screen.getByRole('button', { name: 'claude sessions' }));
    await userEvent.click(await screen.findByRole('option', { name: /Fix the parser/ }));
    expect(importSession).toHaveBeenCalledWith('claude', 's1');
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('new thread menu grows from the sidebar edge into the page, never past the left edge', async () => {
    render(<Sidebar />);
    await userEvent.click(screen.getByRole('button', { name: 'New thread' }));
    const menu = screen.getByRole('menu');
    // Anchored to the sidebar's left edge (not to the right edge of the "+" button) and capped
    // to the viewport, so a wide session list stays on screen.
    expect(menu.className).toMatch(/\bleft-2\b/);
    expect(menu.className).not.toMatch(/\bright-0\b/);
    expect(menu.className).toMatch(/max-w-\[calc\(100vw/);
  });
});

