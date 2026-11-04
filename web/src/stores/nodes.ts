import { create } from 'zustand';
import { api, setNodeBase } from '../api/client';
import { useThreadsStore } from './threads';
import { useFilesStore } from './files';
import type { NodeInfo } from '../api/types';

const KEY = 'octo.node';

function stored(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

function store(id: string | null): void {
  try {
    if (id) localStorage.setItem(KEY, id);
    else localStorage.removeItem(KEY);
  } catch {
    // storage unavailable
  }
}

interface NodesState {
  nodes: NodeInfo[];
  selectedId: string | null;
  load: () => Promise<void>;
  select: (id: string | null) => void;
  selectedOffline: () => boolean;
}

export const useNodesStore = create<NodesState>((set, get) => ({
  nodes: [],
  selectedId: null,
  load: async () => {
    let nodes: NodeInfo[];
    try {
      nodes = await api.nodes();
    } catch {
      return;
    }
    let { selectedId } = get();
    if (selectedId === null) {
      const s = stored();
      if (s && nodes.some((n) => n.node_id === s && !n.is_self)) {
        selectedId = s;
        setNodeBase(s);
      }
    }
    set({ nodes, selectedId });
  },
  select: (id) => {
    const target = id && get().nodes.some((n) => n.node_id === id && !n.is_self) ? id : null;
    if (target === get().selectedId) return;
    setNodeBase(target);
    store(target);
    set({ selectedId: target });
    useThreadsStore.setState({ threads: [], activeId: null });
    useFilesStore.setState({ entries: { private: [], public: [] }, preview: null });
    void useThreadsStore.getState().load().catch(() => {});
    void useThreadsStore.getState().loadProviders().catch(() => {});
  },
  selectedOffline: () => {
    const { nodes, selectedId } = get();
    return nodes.some((n) => n.node_id === selectedId && !n.online);
  },
}));
