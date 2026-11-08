import { create } from 'zustand';
import { api, nodeGeneration } from '../api/client';
import { errMsg } from '../lib/format';
import type { Approval, ProviderKind, ProviderStatus, Space, Thread } from '../api/types';

interface ThreadsState {
  threads: Thread[];
  providers: ProviderStatus[];
  activeId: string | null;
  error: string | null;
  load: () => Promise<void>;
  loadProviders: () => Promise<void>;
  create: (provider: ProviderKind, space: Space, approval: Approval, model?: string) => Promise<void>;
  importSession: (kind: ProviderKind, externalId: string) => Promise<void>;
  setApproval: (id: string, approval: Approval) => Promise<void>;
  remove: (id: string) => Promise<void>;
  select: (id: string | null) => void;
}

export const useThreadsStore = create<ThreadsState>((set, get) => {
  let creating = false;

  /** Runs `f` with the node generation; failures land in `error`, stale outcomes are dropped. */
  const run = async (f: (live: () => boolean) => Promise<void>) => {
    const gen = nodeGeneration();
    const live = () => gen === nodeGeneration();
    try {
      await f(live);
    } catch (e) {
      if (live()) set({ error: errMsg(e) });
    }
  };

  /** Like `run`, but a second call while the first is pending is ignored. */
  const once = async (f: (live: () => boolean) => Promise<void>) => {
    if (creating) return;
    creating = true;
    try {
      await run(f);
    } finally {
      creating = false;
    }
  };

  return {
    threads: [],
    providers: [],
    activeId: null,
    error: null,
    load: () =>
      run(async (live) => {
        const threads = await api.threads();
        if (live()) set({ threads, error: null });
      }),
    loadProviders: () =>
      run(async (live) => {
        const all = await api.providers();
        if (live()) set({ providers: all.filter((p) => p.installed) });
      }),
    create: (provider, space, approval, model) =>
      once(async (live) => {
        const t = await api.createThread(provider, space, approval, model);
        if (live()) set({ threads: [t, ...get().threads], activeId: t.id, error: null });
      }),
    importSession: (kind, externalId) =>
      once(async (live) => {
        const t = await api.importSession(kind, externalId);
        if (!live()) return;
        const rest = get().threads.filter((x) => x.id !== t.id);
        const known = rest.length !== get().threads.length;
        set({ threads: known ? get().threads.map((x) => (x.id === t.id ? t : x)) : [t, ...rest], activeId: t.id, error: null });
      }),
    setApproval: (id, approval) =>
      run(async (live) => {
        const t = await api.setApproval(id, approval);
        if (live()) set({ threads: get().threads.map((x) => (x.id === id ? t : x)), error: null });
      }),
    remove: (id) =>
      run(async (live) => {
        await api.deleteThread(id);
        if (!live()) return;
        const { threads, activeId } = get();
        set({ threads: threads.filter((t) => t.id !== id), activeId: activeId === id ? null : activeId, error: null });
      }),
    select: (id) => set({ activeId: id }),
  };
});
