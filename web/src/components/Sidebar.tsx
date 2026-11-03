import { useEffect, useRef, useState } from 'react';
import { Plus, ShieldCheck, Trash2, Zap } from 'lucide-react';
import { useThreadsStore } from '../stores/threads';
import type { Approval, ProviderKind, Space } from '../api/types';

function ApprovalIcon({ approval, size }: { approval: Approval; size: number }) {
  return approval === 'auto' ? <Zap size={size} /> : <ShieldCheck size={size} />;
}

function NewThreadMenu() {
  const providers = useThreadsStore((s) => s.providers);
  const create = useThreadsStore((s) => s.create);
  const [approval, setApproval] = useState<Approval>('confirm');
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  const pick = (kind: ProviderKind, space: Space) => {
    setOpen(false);
    void create(kind, space, approval);
  };

  const choice = 'px-2 py-0.5 rounded-md text-xs text-slate-600 hover:bg-slate-100 hover:text-slate-800 disabled:opacity-30 disabled:hover:bg-transparent';

  return (
    <div ref={ref} className="relative">
      <button
        aria-label="New thread"
        onClick={() => setOpen((v) => !v)}
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
            <div key={p.kind} className="flex items-center justify-between gap-3 px-2 py-1">
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
          ))}
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
              <span className="text-xs text-slate-400">{t.provider}</span>
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
    </aside>
  );
}
