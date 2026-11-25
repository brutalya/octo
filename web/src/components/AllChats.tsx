import { useEffect, useState } from 'react';
import { ChevronLeft, Lock, Pin, Search } from 'lucide-react';
import { api } from '../api/client';
import { useThreadsStore } from '../stores/threads';
import { pinKey, usePinsStore } from '../stores/pins';
import { useDeviceChats } from '../lib/deviceChats';
import { shortCwd } from '../lib/format';
import type { NodeInfo, ProviderKind, SessionSummary, Thread } from '../api/types';

/** Agents whose own sessions (started outside Octo) can be continued here. */
const OUTSIDE: { kind: ProviderKind; label: string }[] = [
  { kind: 'claude', label: 'from Claude Code' },
  { kind: 'antigravity', label: 'from Antigravity' },
];

type Item =
  | { type: 'chat'; n: NodeInfo | undefined; t: Thread; at: number }
  | { type: 'session'; n: NodeInfo; s: SessionSummary; kind: ProviderKind; label: string; at: number };

type Found = Record<string, { kind: ProviderKind; label: string; s: SessionSummary }[]>;

const DAY = 86_400_000;

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function bucket(at: number, now: number): string {
  const today = startOfDay(now);
  if (at >= today) return 'Today';
  if (at >= today - DAY) return 'Yesterday';
  if (at >= today - 6 * DAY) return 'This week';
  return 'Earlier';
}

function when(at: number, now: number): string {
  const d = new Date(at);
  return at >= startOfDay(now) - DAY
    ? d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

const chip = (on: boolean) =>
  `flex h-8 shrink-0 items-center rounded-full border px-3 text-xs max-md:h-10 ${
    on ? 'border-indigo-500 bg-indigo-50 text-slate-800' : 'border-slate-200 text-slate-600 hover:bg-slate-100'
  }`;

/** Every chat of every reachable device plus agent sessions started outside Octo, newest first. */
export function AllChats({
  device, onBack, onNavigate,
}: {
  device: string | null;
  onBack: () => void;
  onNavigate?: () => void;
}) {
  const { selfId, multi, order, reachable, listOf, goTo } = useDeviceChats();
  const pins = usePinsStore((s) => s.pins);
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState<string>(device ?? 'all');
  const [found, setFound] = useState<Found>({});
  const [now] = useState(Date.now);

  // Outside sessions: asked from each reachable device; a device that refuses (no agents
  // allowed, agent not installed) simply has none.
  const reach = order.filter(reachable).map((n) => n.node_id).join(',');
  useEffect(() => {
    let live = true;
    const q = query.trim();
    const timer = setTimeout(() => {
      const targets = order.filter(reachable);
      void Promise.all(
        targets.map(async (n) => {
          const per = await Promise.all(
            OUTSIDE.map(async ({ kind, label }) => {
              try {
                const list = await api.importSessionsOn(n.is_self ? null : n.node_id, kind, q);
                return list.filter((s) => !s.imported_thread_id).map((s) => ({ kind, label, s }));
              } catch {
                return [];
              }
            }),
          );
          return [n.node_id, per.flat()] as const;
        }),
      ).then((all) => live && setFound(Object.fromEntries(all)));
    }, q ? 200 : 0);
    return () => {
      live = false;
      clearTimeout(timer);
    };
    // `order` changes identity on every render; `reach` is what matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reach, query]);

  const q = query.trim().toLowerCase();
  const inScope = (n: NodeInfo) => scope === 'all' || scope === 'pinned' || scope === n.node_id;
  const items: Item[] = [];
  for (const n of order.filter(inScope)) {
    for (const t of listOf(n.node_id)) {
      if (q && !(t.title || 'untitled').toLowerCase().includes(q)) continue;
      if (scope === 'pinned' && !pins.includes(pinKey(n.node_id, t.id))) continue;
      items.push({ type: 'chat', n, t, at: t.updated_at });
    }
    if (scope !== 'pinned' && reachable(n)) {
      for (const { kind, label, s } of found[n.node_id] ?? []) items.push({ type: 'session', n, s, kind, label, at: s.updated_at });
    }
  }
  if (!multi && items.length === 0 && scope !== 'pinned') {
    // One device and no network: its chats are still listed.
    for (const t of listOf(selfId)) {
      if (!q || (t.title || 'untitled').toLowerCase().includes(q)) items.push({ type: 'chat', n: undefined, t, at: t.updated_at });
    }
  }
  items.sort((a, b) => b.at - a.at);
  const groups: { name: string; items: Item[] }[] = [];
  for (const it of items) {
    const name = bucket(it.at, now);
    const g = groups.at(-1);
    if (g?.name === name) g.items.push(it);
    else groups.push({ name, items: [it] });
  }

  const openChat = (n: NodeInfo | undefined, id: string) => {
    goTo(n);
    useThreadsStore.getState().select(id);
    onNavigate?.();
  };
  const openSession = (n: NodeInfo, kind: ProviderKind, id: string) => {
    goTo(n);
    void useThreadsStore.getState().importSession(kind, id);
    onNavigate?.();
  };

  const row = (it: Item) => {
    const where = multi && it.n ? `${it.n.name} · ` : '';
    if (it.type === 'chat') {
      const { t } = it;
      const pinned = it.n && pins.includes(pinKey(it.n.node_id, t.id));
      return (
        <li key={`c/${it.n?.node_id}/${t.id}`}>
          <button
            aria-label={`Open ${t.title || 'Untitled'}`}
            onClick={() => openChat(it.n, t.id)}
            className="flex min-h-14 w-full items-center gap-3 rounded-lg px-2 text-left hover:bg-slate-100/60"
          >
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex items-center gap-1.5 text-sm text-slate-800">
                {t.space === 'private' && <Lock size={12} role="img" aria-label="Private" className="shrink-0 text-violet-300" />}
                {pinned && <Pin size={12} aria-hidden="true" className="shrink-0 text-indigo-500" />}
                <span className={`truncate ${t.title ? '' : 'italic text-slate-400'}`}>{t.title || 'Untitled'}</span>
              </span>
              <span className="truncate text-xs text-slate-400">
                {where}
                {t.provider}
                {t.cwd ? ` · ${shortCwd(t.cwd)}` : ''}
              </span>
            </span>
            <span className="shrink-0 text-xs text-slate-400">{when(t.updated_at, now)}</span>
          </button>
        </li>
      );
    }
    const { s } = it;
    return (
      <li key={`s/${it.n.node_id}/${it.kind}/${s.external_id}`}>
        <button
          aria-label={`Open ${s.title}`}
          onClick={() => openSession(it.n, it.kind, s.external_id)}
          className="flex min-h-14 w-full items-center gap-3 rounded-lg px-2 text-left hover:bg-slate-100/60"
        >
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex min-w-0 items-center gap-2 text-sm text-slate-800">
              <span className="truncate">{s.title}</span>
              <span className="shrink-0 rounded bg-slate-100 px-1.5 py-0.5 text-[11px] text-slate-600">{it.label}</span>
            </span>
            <span className="truncate text-xs text-slate-400">
              {where}
              {shortCwd(s.cwd)}
            </span>
          </span>
          <span className="shrink-0 text-xs text-slate-400">{when(s.updated_at, now)}</span>
        </button>
      </li>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="flex items-center gap-1">
        <button
          aria-label="Back"
          onClick={onBack}
          className="flex h-10 w-10 items-center justify-center rounded-lg text-slate-600 hover:bg-slate-100 max-md:h-11 max-md:w-11"
        >
          <ChevronLeft size={18} />
        </button>
        <h2 className="text-base font-semibold text-slate-800">All chats</h2>
      </div>
      <label className="flex h-10 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-2.5 text-slate-400 focus-within:border-indigo-500 max-md:h-11">
        <Search size={14} />
        <input
          autoFocus
          type="search"
          aria-label="Search all chats"
          placeholder="Search all chats"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="min-w-0 flex-1 bg-transparent text-sm text-slate-800 outline-none placeholder:text-slate-400"
        />
      </label>
      <div className="-mx-1 flex flex-wrap gap-1.5 px-1">
        {multi && (
          <>
            <button aria-pressed={scope === 'all'} onClick={() => setScope('all')} className={chip(scope === 'all')}>
              All devices
            </button>
            {order.map((n) => (
              <button
                key={n.node_id}
                aria-pressed={scope === n.node_id}
                onClick={() => setScope(n.node_id)}
                className={chip(scope === n.node_id)}
              >
                {n.name}
              </button>
            ))}
          </>
        )}
        <button aria-pressed={scope === 'pinned'} onClick={() => setScope(scope === 'pinned' ? 'all' : 'pinned')} className={chip(scope === 'pinned')}>
          Pinned
        </button>
      </div>
      <nav className="-mx-1 flex-1 overflow-y-auto px-1">
        {groups.length === 0 && <p className="px-2 py-4 text-sm text-slate-400">Nothing found</p>}
        {groups.map((g) => (
          <section key={g.name}>
            <h3 className="px-2 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-slate-400">{g.name}</h3>
            <ul aria-label={g.name} className="flex flex-col gap-0.5">
              {g.items.map(row)}
            </ul>
          </section>
        ))}
      </nav>
    </div>
  );
}
