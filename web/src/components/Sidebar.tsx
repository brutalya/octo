import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, Lock, MoreHorizontal, Pin, Plus, Search, Settings, X } from 'lucide-react';
import { SettingsModal } from './SettingsModal';
import { AllChats } from './AllChats';
import { api } from '../api/client';
import { useThreadsStore } from '../stores/threads';
import { useChatStore } from '../stores/chat';
import { useDevicesStore } from '../stores/devices';
import { useDeviceChats } from '../lib/deviceChats';
import { pinKey, usePinsStore } from '../stores/pins';
import { relativeTime } from '../lib/format';
import { useClickOutside } from '../lib/useClickOutside';
import { Logo } from './Logo';
import type { NetworkView, NodeInfo, Thread } from '../api/types';

const RECENT = 3;
const COLLAPSED = 'octo.collapsed';

function shortModel(id: string): string {
  return (id.split('/').pop() ?? id).replace(/:latest$/, '');
}

function subtitle(t: Thread, running: boolean, now: number): string {
  if (running) return `${t.provider} · running`;
  const model = t.model ? ` · ${shortModel(t.model)}` : '';
  return `${t.provider}${model} · ${relativeTime(t.updated_at, now)}`;
}

function storedCollapsed(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(COLLAPSED) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function ThreadRow({
  t, detail, active, running, pinned, onSelect, onPin, onDelete,
}: {
  t: Thread;
  detail: string;
  active: boolean;
  running: boolean;
  pinned: boolean;
  onSelect: () => void;
  onPin: () => void;
  onDelete: () => void;
}) {
  const [menu, setMenu] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const ref = useRef<HTMLLIElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const close = useCallback(() => {
    setMenu(false);
    setConfirming(false);
  }, []);
  useClickOutside(ref, menu, close);

  // Deleting needs a second tap within 3 s.
  const del = () => {
    clearTimeout(timer.current);
    if (confirming) {
      close();
      onDelete();
      return;
    }
    setConfirming(true);
    timer.current = setTimeout(() => setConfirming(false), 3000);
  };

  return (
    <li ref={ref} className="group relative">
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
          ) : pinned ? (
            <Pin size={12} strokeWidth={2.4} aria-hidden="true" className="text-indigo-500" />
          ) : null}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5 pr-6 max-md:pr-10">
          <span className={`truncate text-sm ${t.title ? 'text-slate-800' : 'italic text-slate-400'}`}>
            {t.title || 'Untitled'}
          </span>
          <span className="truncate text-xs text-slate-400">{detail}</span>
        </span>
      </button>
      <button
        aria-label="Chat actions"
        aria-haspopup="menu"
        aria-expanded={menu}
        onClick={() => setMenu((v) => !v)}
        className="absolute right-1 top-1.5 rounded-md p-1 text-slate-400 opacity-0 hover:text-slate-800 focus:opacity-100 group-hover:opacity-100 max-md:top-1/2 max-md:flex max-md:h-10 max-md:w-10 max-md:-translate-y-1/2 max-md:items-center max-md:justify-center max-md:p-0 max-md:opacity-100"
      >
        <MoreHorizontal size={15} />
      </button>
      {menu && (
        <div
          role="menu"
          aria-label="Chat actions"
          className="absolute right-1 top-full z-20 -mt-1 flex w-44 flex-col rounded-xl border border-slate-200 bg-white p-1 shadow-lg"
        >
          <button
            role="menuitem"
            onClick={() => {
              close();
              onPin();
            }}
            className="flex h-10 items-center rounded-lg px-3 text-left text-sm text-slate-800 hover:bg-slate-100"
          >
            {pinned ? 'Unpin' : 'Pin'}
          </button>
          <button
            role="menuitem"
            onClick={del}
            className="flex h-10 items-center rounded-lg px-3 text-left text-sm text-red-400 hover:bg-red-50"
          >
            {confirming ? 'Confirm delete' : 'Delete'}
          </button>
        </div>
      )}
    </li>
  );
}

const groupTitle = 'px-2 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-slate-400';

export function Sidebar({ onNavigate, onClose }: { onNavigate?: () => void; onClose?: () => void } = {}) {
  const activeId = useThreadsStore((s) => s.activeId);
  const error = useThreadsStore((s) => s.error);
  const streaming = useChatStore((s) => s.streaming);
  const { nodes, self, selfId, openId, multi, order, allowed, listOf, goTo } = useDeviceChats();
  const pins = usePinsStore((s) => s.pins);
  const [settingsOpen, setSettingsOpen] = useState(false);
  /** All chats screen: undefined = closed, null = every device, else that device's chip. */
  const [all, setAll] = useState<string | null | undefined>(undefined);
  const [net, setNet] = useState<NetworkView | null>(null);
  const [collapsed, setCollapsed] = useState<string[]>(storedCollapsed);
  // Ages move on their own, so the clock is state that ticks, not a read during render.
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(tick);
  }, []);

  useEffect(() => {
    api.network().then((n) => setNet(n.joined ? n : null)).catch(() => {});
  }, []);

  // Other devices' chats: on open, when devices come and go, and every 20 s.
  const reach = nodes.map((n) => `${n.node_id}:${n.online}`).join(',');
  useEffect(() => {
    void useDevicesStore.getState().load();
    const tick = setInterval(() => void useDevicesStore.getState().load(), 20_000);
    return () => clearInterval(tick);
  }, [reach]);

  const open = (n: NodeInfo | undefined, id: string) => {
    goTo(n);
    useThreadsStore.getState().select(id);
    onNavigate?.();
  };

  const remove = async (n: NodeInfo | undefined, id: string) => {
    const nodeId = n?.node_id ?? selfId;
    if (pins.includes(pinKey(nodeId, id))) usePinsStore.getState().toggle(pinKey(nodeId, id));
    if (nodeId === openId) {
      await useThreadsStore.getState().remove(id);
      return;
    }
    try {
      await api.deleteThreadOn(n?.is_self ? null : nodeId, id);
      useDevicesStore.setState((s) => ({ lists: { ...s.lists, [nodeId]: (s.lists[nodeId] ?? []).filter((x) => x.id !== id) } }));
    } catch (e) {
      useThreadsStore.setState({ error: e instanceof Error ? e.message : String(e) });
    }
  };

  const toggle = (list: string[], id: string) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]);
  const collapse = (id: string) => {
    const next = toggle(collapsed, id);
    setCollapsed(next);
    try {
      localStorage.setItem(COLLAPSED, JSON.stringify(next));
    } catch {
      // storage unavailable
    }
  };


  const row = (n: NodeInfo | undefined, t: Thread, detail?: string) => {
    const nodeId = n?.node_id ?? selfId;
    const here = nodeId === openId;
    const running = here && streaming[t.id] !== undefined;
    return (
      <ThreadRow
        key={`${nodeId}/${t.id}`}
        t={t}
        detail={detail ?? subtitle(t, running, now)}
        active={here && t.id === activeId}
        running={running}
        pinned={pins.includes(pinKey(nodeId, t.id))}
        onSelect={() => open(n, t.id)}
        onPin={() => usePinsStore.getState().toggle(pinKey(nodeId, t.id))}
        onDelete={() => void remove(n, t.id)}
      />
    );
  };

  const pinned = pins.flatMap((key) => {
        const [nodeId, id] = key.split('/');
        const n = nodes.find((x) => x.node_id === nodeId);
        const t = listOf(nodeId).find((x) => x.id === id);
        if (!t || (multi && (!n || !allowed(n) || (!n.is_self && !n.online)))) return [];
    return [{ n, t }];
  });

  return (
    <aside
      className={`flex h-full shrink-0 flex-col gap-3 border-slate-200 bg-white p-3 ${
        onClose ? 'w-full' : 'w-64 border-r'
      }`}
    >
      <div className="flex items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-col pl-1">
          <span className="flex items-center gap-1.5 text-base font-semibold text-slate-800">
            <Logo className="h-4 w-auto shrink-0" />
            Octo
          </span>
          {net && (
            <span className="truncate text-xs text-slate-500">
              {net.name} · {net.devices} {net.devices === 1 ? 'device' : 'devices'}
            </span>
          )}
        </div>
        <button
          aria-label="New chat"
          onClick={() => {
            useThreadsStore.getState().openMenu(true);
            onNavigate?.();
          }}
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-indigo-500 text-slate-50 hover:bg-indigo-400 max-md:h-11 max-md:w-11"
        >
          <Plus size={18} strokeWidth={2.4} />
        </button>
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
      {all !== undefined ? (
        <AllChats
          device={all}
          onBack={() => setAll(undefined)}
          onNavigate={() => {
            setAll(undefined);
            onNavigate?.();
          }}
        />
      ) : (
        <>
        <button
          onClick={() => setAll(null)}
          className="flex h-9 shrink-0 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-2.5 text-left text-sm text-slate-400 hover:border-slate-300 max-md:h-11"
        >
          <Search size={14} />
          Search all chats
        </button>
        <nav className="-mx-1 flex-1 overflow-y-auto px-1">
          {pinned.length > 0 && (
            <section>
              <h3 className={groupTitle}>Pinned</h3>
              <ul aria-label="Pinned" className="flex flex-col gap-0.5">
                {pinned.map(({ n, t }) => row(n, t, multi && n ? `${n.name} · ${t.provider}` : undefined))}
              </ul>
            </section>
          )}
          {!multi ? (
            <ul aria-label="Chats" className="flex flex-col gap-0.5 pt-1">
              {listOf(selfId).map((t) => row(self, t))}
            </ul>
          ) : (
            order.map((n) => {
              const offline = !n.is_self && !n.online;
              const chats = listOf(n.node_id);
              // This device without chats (a phone runs no agents) is just noise here.
              if (n.is_self && chats.length === 0) return null;
              const shut = collapsed.includes(n.node_id);
              const state = n.is_self ? 'this device' : n.online ? 'online' : 'offline';
              return (
                <section key={n.node_id} className="pt-2">
                  <button
                    aria-label={`${n.name}, ${state}`}
                    aria-expanded={!offline && !shut}
                    disabled={offline}
                    onClick={() => collapse(n.node_id)}
                    className="flex h-9 w-full items-center gap-2 rounded-lg px-2 text-left hover:bg-slate-100/60 disabled:hover:bg-transparent max-md:h-11"
                  >
                    {offline || shut ? (
                      <ChevronRight size={14} className="shrink-0 text-slate-400" />
                    ) : (
                      <ChevronDown size={14} className="shrink-0 text-slate-400" />
                    )}
                    <span className={`truncate text-sm font-semibold ${offline ? 'text-slate-400' : 'text-slate-800'}`}>
                      {n.name}
                    </span>
                    {!n.is_self && n.online && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />}
                    {(n.is_self || offline) && <span className="truncate text-xs text-slate-400">{state}</span>}
                  </button>
                  {!offline && !shut && (
                    <ul aria-label={`Chats on ${n.name}`} className="flex flex-col gap-0.5 pl-3">
                      {chats.slice(0, RECENT).map((t) => row(n, t))}
                      {chats.length > RECENT && (
                        <li>
                          <button
                            onClick={() => setAll(n.node_id)}
                            className="flex h-8 items-center px-2 text-xs text-indigo-500 hover:underline max-md:h-10"
                          >
                            {`All chats on ${n.name} · ${chats.length}`}
                          </button>
                        </li>
                      )}
                    </ul>
                  )}
                </section>
              );
            })
          )}
        </nav>
        </>
      )}
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
