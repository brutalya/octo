import { create } from 'zustand';
import { api } from '../api/client';
import type { Approval, ProviderKind, ProviderStatus, Space, Thread } from '../api/types';

interface ThreadsState {
  threads: Thread[];
  providers: ProviderStatus[];
  activeId: string | null;
  load: () => Promise<void>;
  loadProviders: () => Promise<void>;
  create: (provider: ProviderKind, space: Space, approval: Approval, model?: string) => Promise<void>;
  setApproval: (id: string, approval: Approval) => Promise<void>;
  remove: (id: string) => Promise<void>;
  select: (id: string | null) => void;
}

export const useThreadsStore = create<ThreadsState>((set, get) => ({
  threads: [],
  providers: [],
  activeId: null,
  load: async () => set({ threads: await api.threads() }),
  loadProviders: async () => {
    const all = await api.providers();
    set({ providers: all.filter((p) => p.installed) });
  },
  create: async (provider, space, approval, model) => {
    const t = await api.createThread(provider, space, approval, model);
    set({ threads: [t, ...get().threads], activeId: t.id });
  },
  setApproval: async (id, approval) => {
    const t = await api.setApproval(id, approval);
    set({ threads: get().threads.map((x) => (x.id === id ? t : x)) });
  },
  remove: async (id) => {
    await api.deleteThread(id);
    const { threads, activeId } = get();
    set({ threads: threads.filter((t) => t.id !== id), activeId: activeId === id ? null : activeId });
  },
  select: (id) => set({ activeId: id }),
}));
