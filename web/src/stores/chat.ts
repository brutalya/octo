import { create } from 'zustand';
import { api } from '../api/client';
import type { Message } from '../api/types';
import { useThreadsStore } from './threads';

interface Streaming {
  text: string;
  tool: string | null;
}

interface ChatState {
  messages: Record<string, Message[]>;
  streaming: Record<string, Streaming | undefined>;
  load: (threadId: string) => Promise<void>;
  send: (threadId: string, content: string) => Promise<void>;
  stop: (threadId: string) => Promise<void>;
}

const controllers = new Map<string, AbortController>();
let localId = 0;

function local(threadId: string, role: Message['role'], content: string): Message {
  return { id: `local-${++localId}`, thread_id: threadId, role, content, created_at: Date.now() };
}

export const useChatStore = create<ChatState>((set, get) => {
  const append = (threadId: string, m: Message) =>
    set((s) => ({ messages: { ...s.messages, [threadId]: [...(s.messages[threadId] ?? []), m] } }));
  const setStreaming = (threadId: string, v: Streaming | undefined) =>
    set((s) => ({ streaming: { ...s.streaming, [threadId]: v } }));

  return {
    messages: {},
    streaming: {},

    load: async (threadId) => {
      const msgs = await api.messages(threadId);
      set((s) => ({ messages: { ...s.messages, [threadId]: msgs } }));
    },

    send: async (threadId, content) => {
      if (get().streaming[threadId]) return;
      const ctrl = new AbortController();
      controllers.set(threadId, ctrl);
      append(threadId, local(threadId, 'user', content));
      setStreaming(threadId, { text: '', tool: null });
      let text = '';
      let aborted = false;
      try {
        for await (const e of api.send(threadId, content, ctrl.signal)) {
          if (e.type === 'text_delta') {
            text += e.text;
            setStreaming(threadId, { text, tool: null });
          } else if (e.type === 'tool') {
            setStreaming(threadId, { text, tool: e.summary || e.name });
          } else if (e.type === 'error') {
            if (text) append(threadId, local(threadId, 'assistant', text));
            append(threadId, local(threadId, 'error', e.message));
            text = '';
            break;
          } else if (e.type === 'done') {
            break;
          }
        }
        if (text) append(threadId, local(threadId, 'assistant', text));
      } catch (err) {
        if (ctrl.signal.aborted) aborted = true;
        else {
          if (text) append(threadId, local(threadId, 'assistant', text));
          append(threadId, local(threadId, 'error', err instanceof Error ? err.message : String(err)));
        }
      } finally {
        controllers.delete(threadId);
        setStreaming(threadId, undefined);
      }
      if (aborted) await get().load(threadId).catch(() => {});
      await useThreadsStore.getState().load().catch(() => {});
    },

    stop: async (threadId) => {
      controllers.get(threadId)?.abort();
      await api.cancel(threadId).catch(() => {});
    },
  };
});
