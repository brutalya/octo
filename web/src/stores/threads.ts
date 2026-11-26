import { create } from 'zustand';
import { api, currentNodeId, nodeGeneration, NODE_KEY } from '../api/client';
import { errMsg } from '../lib/format';
import type { Approval, ProviderKind, ProviderStatus, Space, Thread } from '../api/types';

const CACHE = 'octo.threads';

/**
 * The last list of this device, so a reload shows it at once. Public threads only: the UI can be
 * opened from another machine, and Private titles must not be stored there.
 */
function readCache(): Thread[] {
  try {
    if (localStorage.getItem(NODE_KEY)) return []; // another device opens first
    const v: unknown = JSON.parse(localStorage.getItem(CACHE) ?? '[]');
    return Array.isArray(v) ? (v as Thread[]).filter((t) => t && t.space === 'public' && typeof t.id === 'string') : [];
  } catch {
    return [];
  }
}

function writeCache(threads: Thread[]): void {
  try {
    localStorage.setItem(CACHE, JSON.stringify(threads.filter((t) => t.space === 'public')));
  } catch {
    // storage unavailable
  }
}

interface ThreadsState {
  threads: Thread[];
  providers: ProviderStatus[];
  activeId: string | null;
  error: string | null;
  /** The new thread menu; opened from the sidebar or from an empty page. */
  menuOpen: boolean;
  openMenu: (open: boolean) => void;
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

  /** Remembers this device's list; another device's list is not it. */
  const save = () => {
    if (currentNodeId() === null) writeCache(get().threads);
  };

  const cached = readCache();
  return {
    threads: cached,
    providers: [],
    activeId: cached[0]?.id ?? null,
    error: null,
    menuOpen: false,
    openMenu: (open) => set({ menuOpen: open }),
    // The list is newest first; with nothing open the newest thread opens.
    load: () =>
      run(async (live) => {
        const threads = await api.threads();
        if (!live()) return;
        const { activeId } = get();
        const keep = activeId !== null && threads.some((t) => t.id === activeId);
        set({ threads, activeId: keep ? activeId : (threads[0]?.id ?? null), error: null });
        save();
      }),
    loadProviders: () =>
      run(async (live) => {
        const all = await api.providers();
        if (live()) set({ providers: all.filter((p) => p.installed) });
      }),
    create: (provider, space, approval, model) =>
      once(async (live) => {
        const t = await api.createThread(provider, space, approval, model);
        if (!live()) return;
        set({ threads: [t, ...get().threads], activeId: t.id, error: null });
        save();
      }),
    importSession: (kind, externalId) =>
      once(async (live) => {
        const t = await api.importSession(kind, externalId);
        if (!live()) return;
        const rest = get().threads.filter((x) => x.id !== t.id);
        const known = rest.length !== get().threads.length;
        set({ threads: known ? get().threads.map((x) => (x.id === t.id ? t : x)) : [t, ...rest], activeId: t.id, error: null });
        save();
      }),
    setApproval: (id, approval) =>
      run(async (live) => {
        const t = await api.setApproval(id, approval);
        if (!live()) return;
        set({ threads: get().threads.map((x) => (x.id === id ? t : x)), error: null });
        save();
      }),
    remove: (id) =>
      run(async (live) => {
        await api.deleteThread(id);
        if (!live()) return;
        const { threads, activeId } = get();
        const rest = threads.filter((t) => t.id !== id);
        set({ threads: rest, activeId: activeId === id ? (rest[0]?.id ?? null) : activeId, error: null });
        save();
      }),
    select: (id) => set({ activeId: id }),
  };
});
