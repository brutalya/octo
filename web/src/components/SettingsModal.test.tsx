import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SettingsModal } from './SettingsModal';

const settings = { ollama_url: 'http://127.0.0.1:11434', openrouter_api_key: 'sk-...abcd', history_char_budget: 24000 };
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });
let f: ReturnType<typeof vi.fn>;
const patchBody = (): unknown => {
  const c = f.mock.calls.find((x) => (x[1] as RequestInit | undefined)?.method === 'PATCH');
  return c ? JSON.parse((c[1] as RequestInit).body as string) : undefined;
};

beforeEach(() => {
  f = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === '/api/settings' && init?.method === 'PATCH') return json(settings);
    if (url === '/api/settings') return json(settings);
    if (url === '/api/providers/ollama/models') return json([{ id: 'a', name: 'a' }, { id: 'b', name: 'b' }]);
    if (url === '/api/providers/openrouter/models') return json([{ id: 'x', name: 'x' }]);
    return json({}, 404);
  });
  vi.stubGlobal('fetch', f);
});
afterEach(() => vi.unstubAllGlobals());

describe('SettingsModal', () => {
  it('shows ollama url and a masked key, never a full key', async () => {
    render(<SettingsModal onClose={() => {}} />);
    expect(await screen.findByDisplayValue('http://127.0.0.1:11434')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('sk-...abcd')).toBeInTheDocument();
    expect(screen.getByLabelText('OpenRouter key')).toHaveValue('');
  });

  it('ollama check saves the url and shows the models count', async () => {
    render(<SettingsModal onClose={() => {}} />);
    const url = await screen.findByLabelText('Ollama URL');
    await userEvent.clear(url);
    await userEvent.type(url, 'http://h:1');
    await userEvent.click(screen.getByRole('button', { name: 'Check Ollama' }));
    expect(await screen.findByText('2 models')).toBeInTheDocument();
    expect(patchBody()).toEqual({ ollama_url: 'http://h:1' });
    const urls = f.mock.calls.map((c) => c[0]);
    expect(urls.indexOf('/api/providers/ollama/models')).toBeGreaterThan(urls.indexOf('/api/settings', 1));
  });

  it('openrouter check saves a typed key and shows the count', async () => {
    render(<SettingsModal onClose={() => {}} />);
    await userEvent.type(await screen.findByLabelText('OpenRouter key'), 'sk-new');
    await userEvent.click(screen.getByRole('button', { name: 'Check OpenRouter' }));
    expect(await screen.findByText('1 models')).toBeInTheDocument();
    expect(patchBody()).toEqual({ openrouter_api_key: 'sk-new' });
  });

  it('openrouter check without typing does not resend the key', async () => {
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByLabelText('OpenRouter key');
    await userEvent.click(screen.getByRole('button', { name: 'Check OpenRouter' }));
    await screen.findByText('1 models');
    expect(f.mock.calls.some((c) => (c[1] as RequestInit | undefined)?.method === 'PATCH')).toBe(false);
  });

  it('shows an error when the check fails', async () => {
    f.mockImplementation(async (url: string) =>
      url.endsWith('/models') ? new Response('unreachable', { status: 502 }) : json(settings),
    );
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByLabelText('Ollama URL');
    await userEvent.click(screen.getByRole('button', { name: 'Check Ollama' }));
    expect(await screen.findByText('unreachable')).toBeInTheDocument();
  });

  it('closes via button', async () => {
    const onClose = vi.fn();
    render(<SettingsModal onClose={onClose} />);
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('is a modal page that Escape closes', async () => {
    const onClose = vi.fn();
    render(<SettingsModal onClose={onClose} />);
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveAttribute('aria-modal', 'true');
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('renders no inputs until settings loaded', async () => {
    render(<SettingsModal onClose={() => {}} />);
    expect(screen.queryByLabelText('Ollama URL')).toBeNull();
    expect(await screen.findByLabelText('Ollama URL')).toBeInTheDocument();
  });

  it('is a page with sections for models, devices and relay', async () => {
    render(<SettingsModal onClose={() => {}} />);
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    for (const name of ['Models', 'Devices', 'Relay']) {
      expect(within(nav).getByRole('link', { name })).toBeInTheDocument();
      expect(screen.getByRole('heading', { name })).toBeInTheDocument();
    }
  });
});
