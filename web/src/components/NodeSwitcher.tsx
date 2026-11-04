import { useEffect, useRef, useState } from 'react';
import { ChevronUp } from 'lucide-react';
import { useNodesStore } from '../stores/nodes';

export function NodeSwitcher() {
  const nodes = useNodesStore((s) => s.nodes);
  const selectedId = useNodesStore((s) => s.selectedId);
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

  const current = nodes.find((n) => (selectedId ? n.node_id === selectedId : n.is_self));
  const name = current?.name ?? 'Octo';
  const offline = current ? !current.online : false;

  return (
    <div ref={ref} className="relative min-w-0 flex-1">
      <button
        aria-label={`Node: ${name}`}
        onClick={() => setOpen(!open)}
        className={`flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-sm text-slate-700 hover:bg-slate-100 ${
          offline ? 'opacity-50' : ''
        }`}
      >
        <span className="truncate">{name}</span>
        <ChevronUp size={14} className="ml-auto shrink-0 text-slate-400" />
      </button>
      {open && (
        <div
          role="listbox"
          className="absolute bottom-full left-0 z-10 mb-1 w-full min-w-48 rounded-lg border border-slate-200 bg-white p-1 shadow-lg"
        >
          {nodes.map((n) => (
            <button
              key={n.node_id}
              role="option"
              aria-selected={n === current}
              data-online={n.online}
              onClick={() => {
                useNodesStore.getState().select(n.is_self ? null : n.node_id);
                setOpen(false);
              }}
              className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-sm text-slate-700 hover:bg-slate-100"
            >
              <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${n.online ? 'bg-emerald-500' : 'bg-slate-300'}`} />
              <span className="truncate">{n.name}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
