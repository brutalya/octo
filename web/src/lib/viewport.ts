import { useSyncExternalStore } from 'react';

const PHONE_QUERY = '(max-width: 767px)';

function subscribePhone(cb: () => void) {
  if (typeof window.matchMedia !== 'function') return () => {};
  const mq = window.matchMedia(PHONE_QUERY);
  mq.addEventListener('change', cb);
  return () => mq.removeEventListener('change', cb);
}

const phoneNow = () => (typeof window.matchMedia === 'function' ? window.matchMedia(PHONE_QUERY).matches : false);

export function useIsPhone(): boolean {
  return useSyncExternalStore(subscribePhone, phoneNow);
}

function subscribeVv(cb: () => void) {
  const vv = window.visualViewport;
  if (!vv) return () => {};
  vv.addEventListener('resize', cb);
  return () => vv.removeEventListener('resize', cb);
}

/** Visible height (shrinks with the on-screen keyboard), or undefined when unsupported/disabled. */
export function useVisualViewportHeight(enabled: boolean): number | undefined {
  const h = useSyncExternalStore(subscribeVv, () => window.visualViewport?.height);
  return enabled ? h : undefined;
}
