import { create } from 'zustand';
import { api } from '../api/client';
import { useNodesStore } from './nodes';
import type { Thread } from '../api/types';

interface DevicesState {
  /** Last known threads per device (node id), newest first. */
  lists: Record<string, Thread[]>;
  /** Fetches this device and every online member; an offline or failing device keeps its list. */
  load: () => Promise<void>;
}

let inflight: Promise<void> | null = null;

export const useDevicesStore = create<DevicesState>((set, get) => ({
  lists: {},
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
      const lists = { ...get().lists };
      for (const g of got) if (g) lists[g[0]] = g[1];
      set({ lists });
    })().finally(() => {
      inflight = null;
    });
    return inflight;
  },
}));
