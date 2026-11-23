import { useCallback, useRef, useState } from 'react';
import { ChevronsUpDown, Monitor } from 'lucide-react';
import { useNodesStore } from '../stores/nodes';
import { useClickOutside } from '../lib/useClickOutside';

export function NodeSwitcher() {
  const nodes = useNodesStore((s) => s.nodes);
  const selectedId = useNodesStore((s) => s.selectedId);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);
  useClickOutside(ref, open, close);

  const current = nodes.find((n) => (selectedId ? n.node_id === selectedId : n.is_self));
  const name = current?.name ?? 'Octo';
  const offline = current ? !current.online : false;
  const online = nodes.filter((n) => n.online).length;
  const detail = !current || current.is_self ? 'This device' : offline ? 'Offline' : 'Remote';

  return (
    <div ref={ref} className="relative min-w-0 flex-1">
      <button
        aria-label={`Node: ${name}`}
        onClick={() => setOpen(!open)}
        className={`flex w-full items-center gap-2.5 rounded-lg border border-slate-200 bg-slate-100/50 px-2 py-1.5 text-left hover:bg-slate-100 ${
          offline ? 'opacity-50' : ''
        }`}
      >
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-slate-100 text-slate-600">
          <Monitor size={15} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-semibold text-slate-800">{name}</span>
          <span className="truncate text-xs text-slate-400">
            {nodes.length > 1 ? `${detail} · ${online} of ${nodes.length} online` : detail}
          </span>
        </span>
        <ChevronsUpDown size={14} className="shrink-0 text-slate-400" />
      </button>
      {open && (
        <div
          role="listbox"
          className="absolute left-0 top-full z-30 mt-1 w-full min-w-48 rounded-lg border border-slate-200 bg-white p-1 shadow-lg"
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
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-slate-700 hover:bg-slate-100 max-md:min-h-11"
            >
              <span
                role="img"
                aria-label={n.online ? 'online' : 'offline'}
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${n.online ? 'bg-emerald-500' : 'bg-slate-300'}`}
              />
              <span className="truncate">{n.name}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
