import { create } from 'zustand';
import { api, ApiError } from '../api/client';
import { useNodesStore } from './nodes';
import type { AccessView, Thread } from '../api/types';

interface DevicesState {
  /** Last known threads per device (node id), newest first. */
  lists: Record<string, Thread[]>;
  /** What this device may do on each member, as the member last answered. */
  access: Record<string, AccessView>;
  /** When each member last answered (ms). */
  accessAt: Record<string, number>;
  /** Fetches this device and every online member; an offline or failing device keeps its list. */
  load: () => Promise<void>;
}

let inflight: Promise<void> | null = null;
/** Members reachable at the previous load: one that just came online is asked right away. */
let wasOnline = new Set<string>();
/** Access rarely changes: asked when a member comes online, then at most this often. */
const ACCESS_TTL = 5 * 60_000;

export const useDevicesStore = create<DevicesState>((set, get) => ({
  lists: {},
  access: {},
  accessAt: {},
  load: () => {
    inflight ??= (async () => {
      const reachable = useNodesStore.getState().nodes.filter((n) => n.is_self || n.online);
      const got = await Promise.all(
        reachable.map(async (n) => {
          try {
            return [n.node_id, await api.threadsOn(n.is_self ? null : n.node_id)] as const;
          } catch {
            return null;
          }
        }),
      );
      const now = Date.now();
      const { accessAt: at } = get();
      const members = reachable.filter((n) => !n.is_self);
      const stale = members.filter(
        (n) => !wasOnline.has(n.node_id) || !(n.node_id in at) || now - at[n.node_id] >= ACCESS_TTL,
      );
      wasOnline = new Set(members.map((n) => n.node_id));
      const asked = await Promise.all(
        stale.map(async (n) => {
          try {
            return [n.node_id, await api.accessOn(n.node_id)] as const;
          } catch (e) {
            // A device older than 0.2.11 refuses the question itself (403): listed as before,
            // and it still enforces its own rules on every request.
            if (e instanceof ApiError && e.status === 403) {
              return [n.node_id, { agents: true, private_files: false }] as const;
            }
            return null; // unreachable now: asked again on the next load
          }
        }),
      );
      const lists = { ...get().lists };
      for (const g of got) if (g) lists[g[0]] = g[1];
      const access = { ...get().access };
      const accessAt = { ...get().accessAt };
      for (const a of asked) {
        if (!a) continue;
        access[a[0]] = a[1];
        accessAt[a[0]] = now;
      }
      set({ lists, access, accessAt });
    })().finally(() => {
      inflight = null;
    });
    return inflight;
  },
}));
