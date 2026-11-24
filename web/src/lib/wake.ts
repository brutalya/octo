import { useEffect } from 'react';
import { api } from '../api/client';

/** Wakes are spaced at least this far apart (focus and visibility often fire together). */
const MIN_GAP = 2000;

/**
 * When the app comes back to the front (phone unlocked, tab shown, network back), tells the
 * local node to check its relay link at once, then calls `after` so the UI refreshes as soon
 * as the link is good again.
 */
export function useWakeOnResume(after: () => void): void {
  useEffect(() => {
    let last = 0;
    let timers: ReturnType<typeof setTimeout>[] = [];
    const wake = () => {
      if (document.visibilityState !== 'visible') return;
      const now = Date.now();
      if (now - last < MIN_GAP) return;
      last = now;
      void api.wake().catch(() => {});
      after();
      // A dead link is found within ~3 s and replaced right after: refresh again then.
      timers.forEach(clearTimeout);
      timers = [setTimeout(after, 1500), setTimeout(after, 4500)];
    };
    document.addEventListener('visibilitychange', wake);
    window.addEventListener('focus', wake);
    window.addEventListener('online', wake);
    return () => {
      document.removeEventListener('visibilitychange', wake);
      window.removeEventListener('focus', wake);
      window.removeEventListener('online', wake);
      timers.forEach(clearTimeout);
    };
    // `after` is read when the event fires; re-subscribing on every render is not needed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
