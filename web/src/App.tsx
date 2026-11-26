import { useEffect, useState } from 'react';
import { Folder, Menu, Plus, ShieldCheck, Zap } from 'lucide-react';
import { useIsPhone, useVisualViewportHeight } from './lib/viewport';
import { Sidebar } from './components/Sidebar';
import { Feed } from './components/Feed';
import { FilesPanel } from './components/FilesPanel';
import { ThreadHeader } from './components/ThreadHeader';
import { NewThreadDialog } from './components/NewThreadDialog';
import { ChatInput } from './components/ChatInput';
import { useThreadsStore } from './stores/threads';
import { useChatStore } from './stores/chat';
import { useNodesStore } from './stores/nodes';
import { useDevicesStore } from './stores/devices';
import { NODE_KEY } from './api/client';
import { useWakeOnResume } from './lib/wake';

export function App() {
  const activeId = useThreadsStore((s) => s.activeId);
  const active = useThreadsStore((s) => s.threads.find((t) => t.id === s.activeId));
  const space = active?.space;
  const noThreads = useThreadsStore((s) => s.threads.length === 0);
  const offline = useNodesStore((s) => s.nodes.some((n) => n.node_id === s.selectedId && !n.online));
  const remote = useNodesStore((s) => s.nodes.find((n) => n.node_id === s.selectedId && !n.is_self)?.name);
  const streaming = useChatStore((s) => (activeId ? s.streaming[activeId] !== undefined : false));

  const phone = useIsPhone();
  const vvHeight = useVisualViewportHeight(phone);
  const [drawer, setDrawer] = useState(false);
  const [sheet, setSheet] = useState(false);
  const closeDrawer = () => setDrawer(false);

  useEffect(() => {
    // This device's chats do not wait for the node list (unless another device opens first).
    let remoteFirst = false;
    try {
      remoteFirst = localStorage.getItem(NODE_KEY) !== null;
    } catch {
      // storage unavailable
    }
    if (!remoteFirst) {
      void useThreadsStore.getState().load();
      void useThreadsStore.getState().loadProviders();
    }
    void useNodesStore.getState().load().finally(() => {
      // A stored remote device was restored: its data replaces the local list loaded above.
      if (useNodesStore.getState().selectedId === null) return;
      useThreadsStore.setState({ threads: [], activeId: null, error: null });
      void useThreadsStore.getState().load();
      void useThreadsStore.getState().loadProviders();
    });
    const poll = setInterval(() => void useNodesStore.getState().load(), 5000);
    return () => clearInterval(poll);
  }, []);

  // Back in front (phone unlocked): check the link at once and refresh what is shown.
  useWakeOnResume(() => {
    void useNodesStore.getState().load().then(() => {
      void useThreadsStore.getState().load();
      void useDevicesStore.getState().load();
    });
  });

  useEffect(() => {
    if (activeId && !useChatStore.getState().streaming[activeId]) {
      void useChatStore.getState().load(activeId).catch(() => {});
      void useChatStore.getState().loadApprovals(activeId);
    }
  }, [activeId]);

  const start = (
    <div className="flex flex-1 items-center justify-center">
      {noThreads && (
        <button
          onClick={() => useThreadsStore.getState().openMenu(true)}
          className="flex items-center gap-2 rounded-md border border-slate-200 bg-white px-3 py-2 text-sm text-slate-600 hover:border-slate-300 hover:text-slate-800"
        >
          <Plus size={16} />
          Start a thread
        </button>
      )}
    </div>
  );

  const chat = !activeId ? start : (
    <>
      <Feed key={`feed-${activeId}`} threadId={activeId} />
      <ChatInput
        key={`input-${activeId}`}
        space={space}
        title={active?.title}
        streaming={streaming}
        disabled={offline}
        placeholder={active ? `Message ${active.provider}` : undefined}
        onSend={(text, att) => useChatStore.getState().send(activeId, text, att)}
        onStop={() => void useChatStore.getState().stop(activeId)}
      />
    </>
  );

  if (!phone) {
    return (
      <div className="flex h-full bg-slate-50 text-slate-800">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          {active && <ThreadHeader thread={active} />}
          <main className="flex min-h-0 flex-1 flex-col">{chat}</main>
        </div>
        <FilesPanel />
        <NewThreadDialog />
      </div>
    );
  }

  const iconBtn = 'flex h-11 w-11 shrink-0 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-800';
  return (
    <div
      className="flex h-full flex-col bg-slate-50 text-slate-800"
      style={vvHeight ? { height: vvHeight } : undefined}
    >
      <header className="flex h-14 shrink-0 items-center gap-1 border-b border-slate-200 bg-white px-1">
        <button aria-label="Menu" onClick={() => setDrawer(true)} className={iconBtn}>
          <Menu size={20} />
        </button>
        <div className="flex min-w-0 flex-1 flex-col items-center leading-tight">
          <div className="max-w-full truncate text-sm font-semibold text-slate-800">{active ? active.title || 'Untitled' : ''}</div>
          {(remote || active) && (
            <div className="flex max-w-full items-center gap-1.5 truncate text-xs text-slate-400">
              {remote && <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${offline ? 'bg-slate-300' : 'bg-emerald-500'}`} />}
              {[remote && `on ${remote}`, active?.provider].filter(Boolean).join(' · ')}
            </div>
          )}
        </div>
        {active && (
          <button
            aria-label={`Tool approval: ${active.approval === 'auto' ? 'Auto' : 'Ask'}`}
            onClick={() => void useThreadsStore.getState().setApproval(active.id, active.approval === 'auto' ? 'confirm' : 'auto')}
            className="flex h-11 shrink-0 items-center gap-1 rounded-lg px-2 text-xs text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            {active.approval === 'auto' ? <Zap size={14} /> : <ShieldCheck size={14} />}
            {active.approval === 'auto' ? 'Auto' : 'Ask'}
          </button>
        )}
        <button aria-label="Files" onClick={() => setSheet(true)} className={iconBtn}>
          <Folder size={20} />
        </button>
      </header>
      <main className="flex min-h-0 min-w-0 flex-1 flex-col pb-[env(safe-area-inset-bottom)]">{chat}</main>
      {drawer && (
        <div role="dialog" aria-label="Menu" className="fixed inset-0 z-30 flex bg-white pb-[env(safe-area-inset-bottom)]">
          <Sidebar onNavigate={closeDrawer} onClose={closeDrawer} />
        </div>
      )}
      {sheet && <FilesPanel sheet onClose={() => setSheet(false)} />}
      <NewThreadDialog />
    </div>
  );
}
