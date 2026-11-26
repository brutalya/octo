import { create } from 'zustand';
import { api, nodeGeneration, NODE_KEY, setNodeBase } from '../api/client';
import { useThreadsStore } from './threads';
import { useFilesStore } from './files';
import { useAttachStore } from './attach';
import { useChatStore } from './chat';
import type { NodeInfo } from '../api/types';

function stored(): string | null {
  try {
    return localStorage.getItem(NODE_KEY);
  } catch {
    return null;
  }
}

function store(id: string | null): void {
  try {
    if (id) localStorage.setItem(NODE_KEY, id);
    else localStorage.removeItem(NODE_KEY);
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

let inflight: Promise<void> | null = null;

export const useNodesStore = create<NodesState>((set, get) => ({
  nodes: [],
  selectedId: null,
  load: () => {
    inflight ??= (async () => {
      const gen = nodeGeneration();
      let nodes: NodeInfo[];
      try {
        nodes = await api.nodes();
      } catch {
        return;
      }
      if (gen !== nodeGeneration()) return;
      let { selectedId } = get();
      if (selectedId === null) {
        const s = stored();
        if (s && nodes.some((n) => n.node_id === s && !n.is_self)) {
          selectedId = s;
          setNodeBase(s);
        }
      } else if (!nodes.some((n) => n.node_id === selectedId && !n.is_self)) {
        set({ nodes });
        get().select(null);
        return;
      }
      useFilesStore.getState().prune(nodes.map((n) => n.node_id));
      set({ nodes, selectedId });
    })().finally(() => {
      inflight = null;
    });
    return inflight;
  },
  select: (id) => {
    const target = id && get().nodes.some((n) => n.node_id === id && !n.is_self) ? id : null;
    if (target === get().selectedId) return;
    setNodeBase(target);
    store(target);
    set({ selectedId: target });
    useChatStore.getState().reset();
    useThreadsStore.setState({ threads: [], activeId: null, error: null });
    // Picked files are paths on the device that was open.
    useAttachStore.getState().clear();
    void useThreadsStore.getState().load();
    void useThreadsStore.getState().loadProviders();
  },
  selectedOffline: () => {
    const { nodes, selectedId } = get();
    return nodes.some((n) => n.node_id === selectedId && !n.online);
  },
}));
