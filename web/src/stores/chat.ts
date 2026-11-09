import { create } from 'zustand';
import { api, nodeGeneration } from '../api/client';
import { errMsg } from '../lib/format';
import type { AttachmentRef, Message, PendingApproval } from '../api/types';
import { useThreadsStore } from './threads';

interface Streaming {
  text: string;
  tool: string | null;
}

interface ChatState {
  messages: Record<string, Message[]>;
  streaming: Record<string, Streaming | undefined>;
  /** Open approval requests per thread. */
  approvals: Record<string, PendingApproval[]>;
  loadApprovals: (threadId: string) => Promise<void>;
  resolveApproval: (threadId: string, id: string, allow: boolean) => Promise<void>;
  load: (threadId: string) => Promise<void>;
  /** Resolves false when the message was not accepted (nothing was appended). */
  send: (threadId: string, content: string, attachments?: AttachmentRef[]) => Promise<boolean>;
  stop: (threadId: string) => Promise<void>;
  /** Abort every running stream and forget all chat state (node switch). */
  reset: () => void;
}

const controllers = new Map<string, AbortController>();
let localId = 0;

function local(threadId: string, role: Message['role'], content: string): Message {
  return { id: `local-${++localId}`, thread_id: threadId, role, content, attachments: [], created_at: Date.now() };
}

export const useChatStore = create<ChatState>((set, get) => {
  const append = (threadId: string, m: Message) =>
    set((s) => ({ messages: { ...s.messages, [threadId]: [...(s.messages[threadId] ?? []), m] } }));
  const setStreaming = (threadId: string, v: Streaming | undefined) =>
    set((s) => ({ streaming: { ...s.streaming, [threadId]: v } }));

  const setApprovals = (threadId: string, f: (l: PendingApproval[]) => PendingApproval[]) =>
    set((s) => ({ approvals: { ...s.approvals, [threadId]: f(s.approvals[threadId] ?? []) } }));

  return {
    messages: {},
    streaming: {},
    approvals: {},

    loadApprovals: async (threadId) => {
      const gen = nodeGeneration();
      const list = await api.approvals(threadId).catch(() => null);
      if (list && gen === nodeGeneration()) setApprovals(threadId, () => list);
    },

    resolveApproval: async (threadId, id, allow) => {
      await api.resolveApproval(threadId, id, allow);
      setApprovals(threadId, (l) => l.filter((a) => a.id !== id));
    },

    load: async (threadId) => {
      const gen = nodeGeneration();
      const msgs = await api.messages(threadId);
      if (gen === nodeGeneration()) set((s) => ({ messages: { ...s.messages, [threadId]: msgs } }));
    },

    send: async (threadId, content, attachments) => {
      if (get().streaming[threadId]) return false;
      const gen = nodeGeneration();
      const live = () => gen === nodeGeneration();
      const ctrl = new AbortController();
      controllers.set(threadId, ctrl);
      append(threadId, local(threadId, 'user', content));
      setStreaming(threadId, { text: '', tool: null });
      let text = '';
      let sawDone = false;
      let failure: Message | null = null;
      const fail = (message: string) => {
        if (text) append(threadId, local(threadId, 'assistant', text));
        text = '';
        failure = local(threadId, 'error', message);
        append(threadId, failure);
      };
      try {
        for await (const e of api.send(threadId, content, ctrl.signal, attachments)) {
          if (!live()) break;
          if (e.type === 'text_delta') {
            text += e.text;
            setStreaming(threadId, { text, tool: null });
          } else if (e.type === 'tool') {
            setStreaming(threadId, { text, tool: e.summary || e.name });
          } else if (e.type === 'approval') {
            setApprovals(threadId, (l) =>
              l.some((a) => a.id === e.id) ? l : [...l, { id: e.id, tool: e.tool, summary: e.summary, detail: e.detail }],
            );
          } else if (e.type === 'approval_done') {
            setApprovals(threadId, (l) => l.filter((a) => a.id !== e.id));
          } else if (e.type === 'error') {
            sawDone = true;
            fail(e.message);
            break;
          } else if (e.type === 'done') {
            sawDone = true;
            break;
          }
        }
        if (live() && !ctrl.signal.aborted) {
          if (!sawDone) fail('Stream ended unexpectedly');
          else if (text) append(threadId, local(threadId, 'assistant', text));
        }
      } catch (err) {
        if (live() && !ctrl.signal.aborted) fail(errMsg(err));
      } finally {
        if (controllers.get(threadId) === ctrl) controllers.delete(threadId);
        if (live()) {
          setStreaming(threadId, undefined);
          setApprovals(threadId, () => []);
        }
      }
      if (!live()) return true;
      // Replace the local-N messages with the persisted ones, keeping a local error the server did not store.
      try {
        const fresh = await api.messages(threadId);
        if (live()) {
          const err = failure as Message | null;
          const merged = err && !fresh.some((m) => m.role === 'error' && m.content === err.content) ? [...fresh, err] : fresh;
          set((s) => ({ messages: { ...s.messages, [threadId]: merged } }));
        }
      } catch {
        // keep the local messages
      }
      await useThreadsStore.getState().load();
      return true;
    },

    stop: async (threadId) => {
      await api.cancel(threadId).catch(() => {});
      controllers.get(threadId)?.abort();
    },

    reset: () => {
      for (const c of controllers.values()) c.abort();
      controllers.clear();
      set({ messages: {}, streaming: {}, approvals: {} });
    },
  };
});
