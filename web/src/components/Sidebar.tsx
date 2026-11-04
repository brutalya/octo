import { useEffect, useRef, useState } from 'react';
import { Plus, Settings, ShieldCheck, Trash2, Zap } from 'lucide-react';
import { api } from '../api/client';
import { relativeTime, shortCwd } from '../lib/format';
import { SettingsModal } from './SettingsModal';
import { useThreadsStore } from '../stores/threads';
import type { Approval, ModelInfo, ProviderKind, SessionSummary, Space } from '../api/types';

const HTTP_PROVIDERS: ProviderKind[] = ['ollama', 'openrouter'];

export function shortModel(id: string): string {
  return (id.split('/').pop() ?? id).replace(/:latest$/, '');
}

function ModelMenu({ kind, onPick }: { kind: ProviderKind; onPick: (id: string) => void }) {
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    let live = true;
    api.models(kind).then((m) => live && setModels(m)).catch((e: unknown) => live && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [kind]);

  const q = query.trim().toLowerCase();
  const shown = (models ?? []).filter((m) => !q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q));
  return (
    <div className="border-t border-slate-200 p-1">
      <input
        autoFocus
        type="search"
        role="searchbox"
        aria-label="Search models"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        className="mb-1 w-full rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm text-slate-800 outline-none focus:border-indigo-500"
      />
      {error && <div className="px-2 py-1 text-xs text-red-500">{error}</div>}
      <div role="listbox" className="max-h-60 overflow-y-auto">
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
      </div>
    </div>
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
        .catch((e: unknown) => live && setError(e instanceof Error ? e.message : String(e)));
    }, query ? 150 : 0);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [kind, query]);

  return (
    <div className="border-t border-slate-200 p-1">
      <input
        autoFocus
        type="search"
        role="searchbox"
        aria-label="Search sessions"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        className="mb-1 w-full rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm text-slate-800 outline-none focus:border-indigo-500"
      />
      {error && <div className="px-2 py-1 text-xs text-red-500">{error}</div>}
      <div role="listbox" className="max-h-72 w-72 overflow-y-auto">
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
      </div>
    </div>
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
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<{ kind: ProviderKind; space: Space } | null>(null);
  const [sessionsOf, setSessionsOf] = useState<ProviderKind | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const close = () => {
    setOpen(false);
    setPending(null);
    setSessionsOf(null);
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

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
    <div ref={ref} className="relative">
      <button
        aria-label="New thread"
        onClick={() => (open ? close() : setOpen(true))}
        className="p-1.5 rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-800"
      >
        <Plus size={16} />
      </button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 top-full mt-1 z-10 min-w-48 rounded-lg border border-slate-200 bg-white p-1 shadow-lg"
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

export function Sidebar() {
  const threads = useThreadsStore((s) => s.threads);
  const activeId = useThreadsStore((s) => s.activeId);
  const select = useThreadsStore((s) => s.select);
  const remove = useThreadsStore((s) => s.remove);
  const setApproval = useThreadsStore((s) => s.setApproval);
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <aside className="flex h-full w-64 shrink-0 flex-col border-r border-slate-200 bg-white">
      <div className="flex justify-end p-2">
        <NewThreadMenu />
      </div>
      <ul className="flex-1 overflow-y-auto px-2 pb-2">
        {threads.map((t) => (
          <li key={t.id} className="group relative">
            <button
              onClick={() => select(t.id)}
              className={`flex w-full flex-col items-start rounded-md px-2 py-1.5 text-left ${
                t.id === activeId ? 'bg-slate-100' : 'hover:bg-slate-100/60'
              }`}
            >
              <span className="w-full truncate pr-6 text-sm text-slate-800">{t.title || ' '}</span>
              <span className="text-xs text-slate-400">{t.model ? `${t.provider} · ${shortModel(t.model)}` : t.provider}</span>
            </button>
            <button
              aria-label={`Thread approval: ${t.approval}`}
              title={`Approval: ${t.approval}`}
              onClick={() => void setApproval(t.id, t.approval === 'confirm' ? 'auto' : 'confirm')}
              className="absolute bottom-1.5 right-2 rounded p-0.5 text-slate-300 hover:text-slate-600"
            >
              <ApprovalIcon approval={t.approval} size={11} />
            </button>
            <button
              aria-label="Delete thread"
              onClick={() => void remove(t.id)}
              className="absolute right-1 top-1.5 rounded-md p-1 text-slate-400 opacity-0 hover:text-red-500 focus:opacity-100 group-hover:opacity-100"
            >
              <Trash2 size={14} />
            </button>
          </li>
        ))}
      </ul>
      <div className="p-2">
        <button
          aria-label="Settings"
          onClick={() => setSettingsOpen(true)}
          className="rounded-md p-1.5 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
        >
          <Settings size={16} />
        </button>
      </div>
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </aside>
  );
}
