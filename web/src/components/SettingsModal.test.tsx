import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within, fireEvent } from '@testing-library/react';
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
    if (url === '/api/network') return json({ joined: false, name: null, relay_host: null, devices: 1 });
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

  it('is a page with sections for models and the network', async () => {
    render(<SettingsModal onClose={() => {}} />);
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    expect(within(nav).getAllByRole('link').map((a) => a.textContent)).toEqual(['Models', 'Network']);
    expect(screen.getByRole('heading', { name: 'Models' })).toHaveAttribute('id', 'settings-models');
    // The network section's heading follows its state; the link target is always there.
    expect(await screen.findByRole('heading', { name: 'This device is not in a network' })).toHaveAttribute('id', 'settings-network');
  });
});

describe('SettingsModal load states', () => {
  const pending = () => new Promise<Response>(() => {});
  const base = (url: string): Response | null =>
    url === '/api/network' ? json({ joined: false, name: null, relay_host: null, devices: 1 }) : null;

  afterEach(() => vi.useRealTimers());

  it('shows a loading state while settings are pending, not a blank body', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => (url === '/api/settings' ? pending() : base(url) ?? json({}, 404))));
    render(<SettingsModal onClose={() => {}} />);
    expect(await screen.findByRole('status')).toHaveTextContent('Loading');
  });

  it('shows an error with Retry, and Retry refetches and renders the content', async () => {
    let fail = true;
    const g = vi.fn(async (url: string) => {
      if (url === '/api/settings') return fail ? new Response('down', { status: 502 }) : json(settings);
      return base(url) ?? json({}, 404);
    });
    vi.stubGlobal('fetch', g);
    render(<SettingsModal onClose={() => {}} />);
    expect(await screen.findByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.queryByRole('status')).toBeNull();
    fail = false;
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByLabelText('Ollama URL')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it('a hung request times out into the error state', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.stubGlobal('fetch', vi.fn((url: string, init?: RequestInit) =>
      url === '/api/settings'
        ? new Promise<Response>((_, rej) => init?.signal?.addEventListener('abort', () => rej(new Error('timed out'))))
        : Promise.resolve(base(url) ?? json({}, 404)),
    ));
    render(<SettingsModal onClose={() => {}} />);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(await screen.findByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });

  it('refetches when the page becomes visible and settings are not loaded', async () => {
    let calls = 0;
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url === '/api/settings') return ++calls === 1 ? pending() : json(settings);
      return base(url) ?? json({}, 404);
    }));
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByRole('status');
    document.dispatchEvent(new Event('visibilitychange'));
    expect(await screen.findByLabelText('Ollama URL')).toBeInTheDocument();
    expect(calls).toBe(2);
  });

  it('does not refetch on visibility once loaded', async () => {
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByLabelText('Ollama URL');
    const before = f.mock.calls.length;
    document.dispatchEvent(new Event('visibilitychange'));
    expect(f.mock.calls).toHaveLength(before);
  });

  it('marks the active tab: Models first, then the one clicked, then the one scrolled to', async () => {
    render(<SettingsModal onClose={() => {}} />);
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    const models = within(nav).getByRole('link', { name: 'Models' });
    const network = within(nav).getByRole('link', { name: 'Network' });
    expect(models).toHaveAttribute('aria-current', 'true');
    expect(network).not.toHaveAttribute('aria-current');
    await userEvent.click(network);
    expect(network).toHaveAttribute('aria-current', 'true');
    expect(models).not.toHaveAttribute('aria-current');
    const body = screen.getByTestId('settings-body');
    const top = (n: number) => () => ({ top: n }) as DOMRect;
    body.getBoundingClientRect = top(0);
    (document.getElementById('settings-models') as HTMLElement).getBoundingClientRect = top(-50);
    (document.getElementById('settings-network') as HTMLElement).getBoundingClientRect = top(400);
    fireEvent.scroll(body);
    expect(models).toHaveAttribute('aria-current', 'true');
    expect(network).not.toHaveAttribute('aria-current');
  });
});
