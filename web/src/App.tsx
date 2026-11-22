import { useEffect, useState } from 'react';
import { Folder, Menu, X } from 'lucide-react';
import { useIsPhone, useVisualViewportHeight } from './lib/viewport';
import { Sidebar } from './components/Sidebar';
import { Feed } from './components/Feed';
import { FilesPanel } from './components/FilesPanel';
import { ChatInput } from './components/ChatInput';
import { useThreadsStore } from './stores/threads';
import { useChatStore } from './stores/chat';
import { useNodesStore } from './stores/nodes';

export function App() {
  const activeId = useThreadsStore((s) => s.activeId);
  const space = useThreadsStore((s) => s.threads.find((t) => t.id === s.activeId)?.space);
  const offline = useNodesStore((s) => s.nodes.some((n) => n.node_id === s.selectedId && !n.online));
  const streaming = useChatStore((s) => (activeId ? s.streaming[activeId] !== undefined : false));

  const phone = useIsPhone();
  const vvHeight = useVisualViewportHeight(phone);
  const [drawer, setDrawer] = useState(false);
  const [sheet, setSheet] = useState(false);

  useEffect(() => {
    void useNodesStore.getState().load().finally(() => {
      void useThreadsStore.getState().load();
      void useThreadsStore.getState().loadProviders();
    });
    const poll = setInterval(() => void useNodesStore.getState().load(), 5000);
    return () => clearInterval(poll);
  }, []);

  useEffect(() => {
    if (activeId && !useChatStore.getState().streaming[activeId]) {
      void useChatStore.getState().load(activeId).catch(() => {});
      void useChatStore.getState().loadApprovals(activeId);
    }
  }, [activeId]);

  const chat = activeId && (
    <>
      <Feed key={`feed-${activeId}`} threadId={activeId} />
      <ChatInput
        key={`input-${activeId}`}
        space={space}
        streaming={streaming}
        disabled={offline}
        onSend={(text, att) => useChatStore.getState().send(activeId, text, att)}
        onStop={() => void useChatStore.getState().stop(activeId)}
      />
    </>
  );

  if (!phone) {
    return (
      <div className="flex h-full bg-slate-50 text-slate-800">
        <Sidebar />
        <main className="flex min-w-0 flex-1 flex-col">{chat}</main>
        <FilesPanel />
      </div>
    );
  }

  const iconBtn = 'flex h-10 w-10 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-800';
  return (
    <div
      className="flex h-full flex-col bg-slate-50 text-slate-800"
      style={vvHeight ? { height: vvHeight } : undefined}
    >
      <header className="flex shrink-0 items-center justify-between border-b border-slate-200 bg-white px-1">
        <button aria-label="Menu" onClick={() => setDrawer(true)} className={iconBtn}>
          <Menu size={18} />
        </button>
        <button aria-label="Files" onClick={() => setSheet(true)} className={iconBtn}>
          <Folder size={18} />
        </button>
      </header>
      <main className="flex min-h-0 min-w-0 flex-1 flex-col pb-[env(safe-area-inset-bottom)]">{chat}</main>
      {drawer && (
        <div className="fixed inset-0 z-30 flex">
          <div className="relative flex h-full">
            <Sidebar onNavigate={() => setDrawer(false)} />
            <button aria-label="Close menu" onClick={() => setDrawer(false)} className={`absolute left-1 top-1 ${iconBtn}`}>
              <X size={18} />
            </button>
          </div>
          <div className="flex-1 bg-slate-900/30" onClick={() => setDrawer(false)} />
        </div>
      )}
      {sheet && <FilesPanel sheet onClose={() => setSheet(false)} />}
    </div>
  );
}
