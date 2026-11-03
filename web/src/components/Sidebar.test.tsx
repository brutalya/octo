import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
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
  id, title, provider: 'claude', space: 'public', approval: 'confirm', model: null, created_at: 1, updated_at: 1,
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
    await userEvent.click(screen.getAllByRole('button', { name: 'Delete thread' })[0]);
    expect(remove).toHaveBeenCalledWith('a');
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
