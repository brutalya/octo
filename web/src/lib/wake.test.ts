import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useWakeOnResume } from './wake';

let f: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.useFakeTimers();
  f = vi.fn(async () => new Response(null, { status: 204 }));
  vi.stubGlobal('fetch', f);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('useWakeOnResume', () => {
  it('coming back to the front wakes the node once and refreshes, then again after the link check', () => {
    console.log('[Step 1/3] the app is shown again: visibility and focus fire together');
    const after = vi.fn();
    renderHook(() => useWakeOnResume(after));
    document.dispatchEvent(new Event('visibilitychange'));
    window.dispatchEvent(new Event('focus'));
    console.log('[Step 2/3] one wake to the local node, one refresh now');
    expect(f).toHaveBeenCalledTimes(1);
    expect(f.mock.calls[0][0]).toBe('/api/network/wake');
    expect((f.mock.calls[0][1] as RequestInit).method).toBe('POST');
    expect(after).toHaveBeenCalledTimes(1);
    console.log('[Step 3/3] refreshed again while a dead link is being replaced');
    vi.advanceTimersByTime(5000);
    expect(after).toHaveBeenCalledTimes(3);
  });

  it('a hidden page does not wake anything', () => {
    const after = vi.fn();
    const spy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    renderHook(() => useWakeOnResume(after));
    document.dispatchEvent(new Event('visibilitychange'));
    expect(f).not.toHaveBeenCalled();
    expect(after).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  it('the network coming back wakes too, later again', () => {
    const after = vi.fn();
    renderHook(() => useWakeOnResume(after));
    window.dispatchEvent(new Event('online'));
    vi.advanceTimersByTime(10_000);
    window.dispatchEvent(new Event('online'));
    expect(f).toHaveBeenCalledTimes(2);
  });
});
