import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { api, setNodeBase } from './client';

let f: ReturnType<typeof vi.fn>;
beforeEach(() => {
  f = vi.fn(async () => new Response('[]', { status: 200 }));
  vi.stubGlobal('fetch', f);
  setNodeBase(null);
});
afterEach(() => vi.unstubAllGlobals());
const urls = () => f.mock.calls.map((c) => c[0]);

describe('api base', () => {
  it('uses /api locally', async () => {
    await api.threads();
    expect(urls()).toEqual(['/api/threads']);
  });

  it('re-targets data calls to the remote node', async () => {
    setNodeBase('abc');
    await api.threads();
    await api.messages('t1');
    await api.providers();
    await api.models('ollama');
    await api.importSessions('claude', 'q');
    await api.files('public');
    await api.createThread('claude', 'public', 'confirm');
    expect(urls()).toEqual([
      '/api/nodes/abc/threads',
      '/api/nodes/abc/threads/t1/messages',
      '/api/nodes/abc/providers',
      '/api/nodes/abc/providers/ollama/models',
      '/api/nodes/abc/import/claude/sessions?q=q',
      '/api/nodes/abc/files/public',
      '/api/nodes/abc/threads',
    ]);
  });

  it('settings, nodes and pairing stay local', async () => {
    f.mockImplementation(async () => new Response('{}', { status: 200 }));
    setNodeBase('abc');
    await api.settings();
    await api.patchSettings({ node_name: 'x' });
    await api.nodes();
    await api.pairingPending();
    await api.pairingInvite();
    await api.pairingJoin('ws://r#t', 'me');
    await api.pairingConfirm();
    await api.pairingReject();
    // Relay status and check are this device's link (the tunnel refuses them).
    await api.relayStatus();
    await api.relayCheck('wss://relay.example.com/relay');
    await api.pairingState();
    await api.pairingCancel();
    expect(urls()).toEqual([
      '/api/settings', '/api/settings', '/api/nodes', '/api/pairing/pending',
      '/api/pairing/invite', '/api/pairing/join', '/api/pairing/confirm', '/api/pairing/reject',
      '/api/relay', '/api/relay/check', '/api/pairing', '/api/pairing/cancel',
    ]);
  });

  it('send streams via the remote base', async () => {
    setNodeBase('abc');
    f.mockImplementation(async () => new Response('', { status: 200 }));
    const ctl = new AbortController();
    await api.send('t1', 'hi', ctl.signal).next().catch(() => {});
    expect(urls()).toEqual(['/api/nodes/abc/threads/t1/messages']);
  });
});

describe('api errors', () => {
  it('prefers the JSON error field and truncates', async () => {
    f.mockResolvedValueOnce(new Response(JSON.stringify({ error: 'nope' }), { status: 400 }));
    await expect(api.threads()).rejects.toThrow('nope');
    f.mockResolvedValueOnce(new Response('x'.repeat(1000), { status: 500 }));
    await expect(api.threads()).rejects.toHaveProperty('message', 'x'.repeat(300));
  });

  it('nodes tolerates a non-array body', async () => {
    f.mockResolvedValueOnce(new Response('{}', { status: 200 }));
    expect(await api.nodes()).toEqual([]);
  });
});
