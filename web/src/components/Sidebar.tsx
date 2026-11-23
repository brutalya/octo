import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Check, Lock, Plus, Search, Settings, ShieldCheck, Trash2, X, Zap } from 'lucide-react';
import { api } from '../api/client';
import { errMsg, relativeTime, shortCwd } from '../lib/format';
import { useClickOutside } from '../lib/useClickOutside';
import { NodeSwitcher } from './NodeSwitcher';
import { SettingsModal } from './SettingsModal';
import { useThreadsStore } from '../stores/threads';
import { useChatStore } from '../stores/chat';
import type { Approval, ModelInfo, ProviderKind, SessionSummary, Space, Thread } from '../api/types';

const HTTP_PROVIDERS: ProviderKind[] = ['ollama', 'openrouter'];

function shortModel(id: string): string {
  return (id.split('/').pop() ?? id).replace(/:latest$/, '');
}

function SearchList({
  label, query, onQuery, error, listClass, children,
}: {
  label: string;
  query: string;
  onQuery: (q: string) => void;
  error: string | null;
  listClass: string;
  children: ReactNode;
}) {
  return (
    <div className="border-t border-slate-200 p-1">
      <input
        autoFocus
        type="search"
        role="searchbox"
        aria-label={label}
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        className="mb-1 w-full rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm text-slate-800 outline-none focus:border-indigo-500"
      />
      {error && <div className="px-2 py-1 text-xs text-red-500">{error}</div>}
      <div role="listbox" className={`overflow-y-auto ${listClass}`}>
        {children}
      </div>
    </div>
  );
}

function ModelMenu({ kind, onPick }: { kind: ProviderKind; onPick: (id: string) => void }) {
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    let live = true;
    api.models(kind).then((m) => live && setModels(m)).catch((e: unknown) => live && setError(errMsg(e)));
    return () => {
      live = false;
    };
  }, [kind]);

  const q = query.trim().toLowerCase();
  const shown = (models ?? []).filter((m) => !q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q));
  return (
    <SearchList label="Search models" query={query} onQuery={setQuery} error={error} listClass="max-h-60">
      {shown.map((m) => (
        <button
          key={m.id}
          role="option"
          aria-selected={false}
          onClick={() => onPick(m.id)}
          className="block w-full truncate rounded px-2 py-1 text-left text-sm text-slate-700 hover:bg-slate-100"
        >
          {m.name}
        </button>
      ))}
    </SearchList>
  );
}

function SessionMenu({ kind, onPick }: { kind: ProviderKind; onPick: (externalId: string) => void }) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      api.importSessions(kind, query.trim())
        .then((s) => live && setSessions(s))
        .catch((e: unknown) => live && setError(errMsg(e)));
    }, query ? 150 : 0);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [kind, query]);

  return (
    <SearchList label="Search sessions" query={query} onQuery={setQuery} error={error} listClass="max-h-72 w-72">
      {(sessions ?? []).map((s) => (
        <button
          key={s.external_id}
          role="option"
          aria-selected={false}
          onClick={() => onPick(s.external_id)}
          className="block w-full rounded px-2 py-1 text-left hover:bg-slate-100"
        >
          <span className="block truncate text-sm text-slate-700">{s.title}</span>
          <span className="flex justify-between gap-2 text-xs text-slate-400">
            <span className="truncate">{shortCwd(s.cwd)}</span>
            <span className="shrink-0">{relativeTime(s.updated_at)}</span>
          </span>
        </button>
      ))}
    </SearchList>
  );
}

function ApprovalIcon({ approval, size }: { approval: Approval; size: number }) {
  return approval === 'auto' ? <Zap size={size} /> : <ShieldCheck size={size} />;
}

function NewThreadMenu() {
  const providers = useThreadsStore((s) => s.providers);
  const create = useThreadsStore((s) => s.create);
  const importSession = useThreadsStore((s) => s.importSession);
  const [approval, setApproval] = useState<Approval>('confirm');
  const open = useThreadsStore((s) => s.menuOpen);
  const setOpen = useThreadsStore((s) => s.openMenu);
  const [pending, setPending] = useState<{ kind: ProviderKind; space: Space } | null>(null);
  const [sessionsOf, setSessionsOf] = useState<ProviderKind | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const close = useCallback(() => {
    setOpen(false);
    setPending(null);
    setSessionsOf(null);
  }, [setOpen]);
  useClickOutside(ref, open, close);

  const pick = (kind: ProviderKind, space: Space) => {
    setSessionsOf(null);
    if (HTTP_PROVIDERS.includes(kind)) {
      setPending({ kind, space });
      return;
    }
    close();
    void create(kind, space, approval);
  };

  const pickModel = (model: string) => {
    if (!pending) return;
    close();
    void create(pending.kind, pending.space, approval, model);
  };

  const pickSession = (externalId: string) => {
    if (!sessionsOf) return;
    const kind = sessionsOf;
    close();
    void importSession(kind, externalId);
  };

  const choice = 'px-2 py-0.5 rounded-md text-xs text-slate-600 hover:bg-slate-100 hover:text-slate-800 disabled:opacity-30 disabled:hover:bg-transparent';

  return (
    // The menu opens from the sidebar's left edge towards the page, so a wide session list never
    // leaves the screen.
    <div ref={ref} className="relative">
      <button
        onClick={() => (open ? close() : setOpen(true))}
        className="flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-indigo-500 text-sm font-semibold text-slate-50 hover:bg-indigo-400 max-md:h-12"
      >
        <Plus size={16} strokeWidth={2.4} />
        New thread
      </button>
      {open && (
        <div
          role="menu"
          className="absolute left-0 top-full z-20 mt-1 w-80 max-w-[calc(100vw-1.5rem)] rounded-lg border border-slate-200 bg-white p-1 shadow-lg"
        >
          <div className="flex justify-end px-2 pb-1">
            <button
              aria-label={`Approval: ${approval}`}
              title={`Approval: ${approval}`}
              onClick={() => setApproval(approval === 'confirm' ? 'auto' : 'confirm')}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-700"
            >
              <ApprovalIcon approval={approval} size={14} />
            </button>
          </div>
          {providers.map((p) => (
            <div key={p.kind}>
            <div className="flex items-center justify-between gap-3 px-2 py-1">
              <span className="text-sm text-slate-800">{p.kind}</span>
              <span className="flex gap-1">
                <button
                  aria-label={`${p.kind} private`}
                  disabled={!p.is_local}
                  onClick={() => pick(p.kind, 'private')}
                  className={choice}
                >
                  private
                </button>
                <button aria-label={`${p.kind} public`} onClick={() => pick(p.kind, 'public')} className={choice}>
                  public
                </button>
              </span>
            </div>
            {!HTTP_PROVIDERS.includes(p.kind) && (
              <button
                aria-label={`${p.kind} sessions`}
                onClick={() => {
                  setPending(null);
                  setSessionsOf(sessionsOf === p.kind ? null : p.kind);
                }}
                className="block w-full px-2 pb-1 text-left text-xs text-slate-400 hover:text-slate-700"
              >
                sessions
              </button>
            )}
            </div>
          ))}
          {sessionsOf && <SessionMenu key={sessionsOf} kind={sessionsOf} onPick={pickSession} />}
          {pending && <ModelMenu key={`${pending.kind}`} kind={pending.kind} onPick={pickModel} />}
        </div>
      )}
    </div>
  );
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
      <NewThreadMenu />
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
