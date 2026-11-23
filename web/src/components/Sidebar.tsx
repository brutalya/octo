import { useEffect, useRef, useState } from 'react';
import { Check, Lock, Plus, Search, Settings, Trash2, X } from 'lucide-react';
import { NodeSwitcher } from './NodeSwitcher';
import { SettingsModal } from './SettingsModal';
import { useThreadsStore } from '../stores/threads';
import { useChatStore } from '../stores/chat';
import type { Thread } from '../api/types';

function shortModel(id: string): string {
  return (id.split('/').pop() ?? id).replace(/:latest$/, '');
}

function startOfToday(now: number): number {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function subtitle(t: Thread, running: boolean): string {
  if (running) return `${t.provider} · running`;
  return t.model ? `${t.provider} · ${shortModel(t.model)}` : t.provider;
}

function ThreadRow({
  t, active, running, confirming, onSelect, onDelete,
}: {
  t: Thread;
  active: boolean;
  running: boolean;
  confirming: boolean;
  onSelect: () => void;
  onDelete: () => void;
}) {
  return (
    <li className="group relative">
      <button
        aria-label={t.title || 'Untitled'}
        aria-current={active}
        onClick={onSelect}
        className={`flex w-full items-start gap-2.5 rounded-lg px-2 py-2 text-left max-md:min-h-14 max-md:items-center ${
          active ? 'bg-slate-100' : 'hover:bg-slate-100/60'
        }`}
      >
        <span className="mt-1.5 flex w-3 shrink-0 justify-center max-md:mt-0">
          {running ? (
            <span className="h-2 w-2 rounded-full bg-indigo-500" />
          ) : t.space === 'private' ? (
            <Lock size={12} strokeWidth={2.4} role="img" aria-label="Private" className="text-violet-300" />
          ) : null}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5 pr-6 max-md:pr-10">
          <span className={`truncate text-sm ${t.title ? 'text-slate-800' : 'italic text-slate-400'}`}>
            {t.title || 'Untitled'}
          </span>
          <span className="truncate text-xs text-slate-400">{subtitle(t, running)}</span>
        </span>
      </button>
      <button
        aria-label="Delete thread"
        onClick={onDelete}
        className="absolute right-1 top-1.5 rounded-md p-1 text-slate-400 opacity-0 max-md:top-1/2 max-md:flex max-md:h-10 max-md:w-10 max-md:-translate-y-1/2 max-md:items-center max-md:justify-center max-md:p-0 max-md:opacity-100 hover:text-red-500 focus:opacity-100 group-hover:opacity-100"
      >
        {confirming ? <Check size={14} /> : <Trash2 size={14} />}
      </button>
    </li>
  );
}

const groupTitle = 'px-2 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-slate-400';

export function Sidebar({ onNavigate, onClose }: { onNavigate?: () => void; onClose?: () => void } = {}) {
  const threads = useThreadsStore((s) => s.threads);
  const activeId = useThreadsStore((s) => s.activeId);
  const select = useThreadsStore((s) => s.select);
  const remove = useThreadsStore((s) => s.remove);
  const error = useThreadsStore((s) => s.error);
  const streaming = useChatStore((s) => s.streaming);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  // "Today" moves at midnight, so the clock is state that ticks, not a read during render.
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(tick);
  }, []);
  const confirmTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(confirmTimer.current), []);

  // Deleting needs a second tap on the same button within 3 s.
  const askDelete = (id: string) => {
    clearTimeout(confirmTimer.current);
    if (confirmId === id) {
      setConfirmId(null);
      void remove(id);
      return;
    }
    setConfirmId(id);
    confirmTimer.current = setTimeout(() => setConfirmId(null), 3000);
  };

  const q = query.trim().toLowerCase();
  const shown = q ? threads.filter((t) => (t.title || 'untitled').toLowerCase().includes(q)) : threads;
  const today = startOfToday(now);
  const groups = [
    { name: 'Today', items: shown.filter((t) => t.updated_at >= today) },
    { name: 'Earlier', items: shown.filter((t) => t.updated_at < today) },
  ].filter((g) => g.items.length > 0);

  const row = (t: Thread) => (
    <ThreadRow
      key={t.id}
      t={t}
      active={t.id === activeId}
      running={streaming[t.id] !== undefined}
      confirming={confirmId === t.id}
      onSelect={() => {
        select(t.id);
        onNavigate?.();
      }}
      onDelete={() => askDelete(t.id)}
    />
  );

  return (
    <aside
      className={`flex h-full shrink-0 flex-col gap-3 border-slate-200 bg-white p-3 ${
        onClose ? 'w-full' : 'w-64 border-r'
      }`}
    >
      <div className="flex items-center gap-2">
        <NodeSwitcher />
        {onClose && (
          <button
            aria-label="Close menu"
            onClick={onClose}
            className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            <X size={18} />
          </button>
        )}
      </div>
      <button
        onClick={() => {
          useThreadsStore.getState().openMenu(true);
          onNavigate?.();
        }}
        className="flex h-10 w-full shrink-0 items-center justify-center gap-2 rounded-lg bg-indigo-500 text-sm font-semibold text-slate-50 hover:bg-indigo-400 max-md:h-12"
      >
        <Plus size={16} strokeWidth={2.4} />
        New thread
      </button>
      <label className="flex h-9 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-2.5 text-slate-400 focus-within:border-indigo-500 max-md:h-11">
        <Search size={14} />
        <input
          type="search"
          aria-label="Search threads"
          placeholder="Search threads"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="min-w-0 flex-1 bg-transparent text-sm text-slate-800 outline-none placeholder:text-slate-400"
        />
      </label>
      <nav className="-mx-1 flex-1 overflow-y-auto px-1">
        {groups.map((g) => (
          <section key={g.name}>
            <h3 className={groupTitle}>{g.name}</h3>
            <ul aria-label={g.name} className="flex flex-col gap-0.5">
              {g.items.map(row)}
            </ul>
          </section>
        ))}
      </nav>
      {error && <div className="px-1 text-xs text-red-500">{error}</div>}
      <button
        onClick={() => setSettingsOpen(true)}
        className="flex h-10 items-center gap-2.5 rounded-lg px-2.5 text-sm text-slate-600 hover:bg-slate-100 hover:text-slate-800 max-md:h-12"
      >
        <Settings size={16} />
        Settings
      </button>
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </aside>
  );
}
