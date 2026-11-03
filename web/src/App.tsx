import { useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { Feed } from './components/Feed';
import { FilesPanel } from './components/FilesPanel';
import { ChatInput } from './components/ChatInput';
import { useThreadsStore } from './stores/threads';
import { useChatStore } from './stores/chat';

export function App() {
  const activeId = useThreadsStore((s) => s.activeId);
  const space = useThreadsStore((s) => s.threads.find((t) => t.id === s.activeId)?.space);
  const streaming = useChatStore((s) => (activeId ? s.streaming[activeId] !== undefined : false));

  useEffect(() => {
    void useThreadsStore.getState().load().catch(() => {});
    void useThreadsStore.getState().loadProviders().catch(() => {});
  }, []);

  useEffect(() => {
    if (activeId && !useChatStore.getState().streaming[activeId]) {
      void useChatStore.getState().load(activeId).catch(() => {});
    }
  }, [activeId]);

  return (
    <div className="flex h-full bg-slate-50 text-slate-800">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col">
        {activeId && (
          <>
            <Feed threadId={activeId} />
            <ChatInput
              key={activeId}
              space={space}
              streaming={streaming}
              onSend={(text, att) => void useChatStore.getState().send(activeId, text, att)}
              onStop={() => void useChatStore.getState().stop(activeId)}
            />
          </>
        )}
      </main>
      <FilesPanel />
    </div>
  );
}
