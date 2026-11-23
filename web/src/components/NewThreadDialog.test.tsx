import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { NewThreadDialog } from './NewThreadDialog';
import { useThreadsStore } from '../stores/threads';
import type { ProviderStatus } from '../api/types';

const providers: ProviderStatus[] = [
  { kind: 'claude', installed: true, version: '1', is_local: false },
  { kind: 'antigravity', installed: true, version: '1', is_local: true },
  { kind: 'ollama', installed: true, version: null, is_local: true },
  { kind: 'openrouter', installed: true, version: null, is_local: false },
];
const models = [
  { id: 'llama3:8b', name: 'Llama 3 8B' },
  { id: 'qwen2:7b', name: 'Qwen 2 7B' },
];
const sessions = [
  { external_id: 's1', title: 'Fix the parser', cwd: '/srv/apps/demo', updated_at: Date.now() - 3 * 3_600_000, messages: 4, imported_thread_id: null },
  { external_id: 's2', title: 'Write docs', cwd: 'C:\\work\\site', updated_at: Date.now() - 2 * 86_400_000, messages: 2, imported_thread_id: null },
];

let fetchMock: ReturnType<typeof vi.fn>;
const create = vi.fn(async () => {});
const importSession = vi.fn(async () => {});

beforeEach(() => {
  vi.clearAllMocks();
  fetchMock = vi.fn(async (url: string) => {
    if (url.includes('/sessions')) {
      const q = new URL(url, 'http://x').searchParams.get('q') ?? '';
      return new Response(JSON.stringify(sessions.filter((s) => s.title.toLowerCase().includes(q.toLowerCase()))));
    }
    return new Response(JSON.stringify(models));
  });
  vi.stubGlobal('fetch', fetchMock);
  useThreadsStore.setState({ providers, menuOpen: true, create, importSession });
});
afterEach(() => vi.unstubAllGlobals());

const dialog = () => screen.getByRole('dialog', { name: 'New thread' });
const assistant = (name: RegExp) => within(screen.getByRole('radiogroup', { name: 'Assistant' })).getByRole('radio', { name });
const space = (name: RegExp) => within(screen.getByRole('radiogroup', { name: 'Space' })).getByRole('radio', { name });

describe('NewThreadDialog', () => {
  it('is shown only while the menu is open', () => {
    useThreadsStore.setState({ menuOpen: false });
    const { rerender } = render(<NewThreadDialog />);
    expect(screen.queryByRole('dialog')).toBeNull();
    useThreadsStore.setState({ menuOpen: true });
    rerender(<NewThreadDialog />);
    expect(dialog()).toBeInTheDocument();
  });

  it('lists only installed assistants with a plain description', () => {
    useThreadsStore.setState({ providers: [providers[0], { ...providers[1], installed: false }] });
    render(<NewThreadDialog />);
    expect(assistant(/Claude Code/)).toHaveAccessibleDescription(/Agent/);
    expect(screen.queryByRole('radio', { name: /Antigravity/ })).toBeNull();
  });

  it('private is offered only with a local assistant and falls back to public', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(assistant(/Antigravity/));
    await userEvent.click(space(/Private/));
    expect(space(/Private/)).toBeChecked();
    await userEvent.click(assistant(/Claude Code/));
    expect(space(/Private/)).toBeDisabled();
    expect(space(/Public/)).toBeChecked();
  });

  it('starts a thread with the chosen assistant, space and approval, then closes', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(assistant(/Antigravity/));
    await userEvent.click(space(/Private/));
    await userEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(create).toHaveBeenCalledWith('antigravity', 'private', 'confirm', undefined);
    expect(useThreadsStore.getState().menuOpen).toBe(false);
  });

  it('unticking ask before commands starts in auto', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(screen.getByRole('checkbox', { name: /Ask before running commands/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(create).toHaveBeenCalledWith('claude', 'public', 'auto', undefined);
  });

  it('a model assistant needs a model; the list is searchable', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(assistant(/Ollama/));
    expect(screen.getByRole('button', { name: 'Start' })).toBeDisabled();
    expect(await screen.findByRole('option', { name: /Llama 3 8B/ })).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search models' }), 'qwen');
    expect(screen.queryByRole('option', { name: /Llama/ })).toBeNull();
    await userEvent.click(screen.getByRole('option', { name: /Qwen 2 7B/ }));
    await userEvent.click(space(/Private/));
    await userEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(create).toHaveBeenCalledWith('ollama', 'private', 'confirm', 'qwen2:7b');
  });

  it('agents can continue an existing session; search asks the server', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(screen.getByRole('button', { name: 'Continue a Claude Code session' }));
    expect(await screen.findByRole('option', { name: /Fix the parser/ })).toBeInTheDocument();
    expect(screen.getByText('apps/demo')).toBeInTheDocument();
    expect(screen.getByText('3h')).toBeInTheDocument();
    expect(fetchMock.mock.calls.at(-1)?.[0]).toBe('/api/import/claude/sessions?q=');
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search sessions' }), 'docs');
    await waitFor(() => expect(screen.queryByRole('option', { name: /Fix the parser/ })).toBeNull());
    await userEvent.click(screen.getByRole('option', { name: /Write docs/ }));
    expect(importSession).toHaveBeenCalledWith('claude', 's2');
    expect(useThreadsStore.getState().menuOpen).toBe(false);
  });

  it('model assistants offer no sessions', async () => {
    render(<NewThreadDialog />);
    await userEvent.click(assistant(/OpenRouter/));
    expect(screen.queryByRole('button', { name: /Continue a/ })).toBeNull();
  });

  it('Escape and the close button close it', async () => {
    render(<NewThreadDialog />);
    await userEvent.keyboard('{Escape}');
    expect(useThreadsStore.getState().menuOpen).toBe(false);
    useThreadsStore.setState({ menuOpen: true });
    await userEvent.click(await screen.findByRole('button', { name: 'Close' }));
    expect(useThreadsStore.getState().menuOpen).toBe(false);
  });
});
