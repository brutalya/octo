import { describe, it, expect, vi, afterEach } from 'vitest';
import { api } from './client';

afterEach(() => {
  delete (window as unknown as { __OCTO_TOKEN__?: string }).__OCTO_TOKEN__;
  vi.restoreAllMocks();
});

function mockFetch() {
  const fn = vi.fn(async () => new Response('[]', { status: 200 }));
  vi.stubGlobal('fetch', fn);
  return fn;
}

const authHeader = (fn: ReturnType<typeof mockFetch>) => {
  const init = (fn.mock.calls[0] as unknown as [string, RequestInit])[1];
  return new Headers(init?.headers).get('authorization');
};

describe('api token', () => {
  it('sends bearer token when window.__OCTO_TOKEN__ is set', async () => {
    (window as unknown as { __OCTO_TOKEN__?: string }).__OCTO_TOKEN__ = 'abc';
    const fn = mockFetch();
    await api.providers();
    expect(authHeader(fn)).toBe('Bearer abc');
  });

  it('keeps other headers when adding the token', async () => {
    (window as unknown as { __OCTO_TOKEN__?: string }).__OCTO_TOKEN__ = 'abc';
    const fn = mockFetch();
    await api.setApproval('t1', 'auto');
    const init = (fn.mock.calls[0] as unknown as [string, RequestInit])[1];
    expect(new Headers(init.headers).get('content-type')).toBe('application/json');
    expect(authHeader(fn)).toBe('Bearer abc');
  });

  it('sends no authorization without a token', async () => {
    const fn = mockFetch();
    await api.providers();
    expect(authHeader(fn)).toBeNull();
  });
});
