import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, Copy, Eye, EyeOff, Lock, LogIn, Monitor, MoreHorizontal, Network as NetworkIcon, Plus, SquareTerminal } from 'lucide-react';
import { api } from '../api/client';
import { useNodesStore } from '../stores/nodes';
import { errMsg } from '../lib/format';
import { classifyCode, codeHost, joinRelay, relayFromCode, splitRelay } from '../lib/relay';
import { useClickOutside } from '../lib/useClickOutside';
import type { NetworkView, PairingPending, RelayStatus, SettingsView } from '../api/types';

type Mode =
  | { kind: 'idle' }
  | { kind: 'invite'; code: string; expires: number }
  | { kind: 'joining' };

/** Result of the last "Check", shown until the relay fields change. */
type Checked = { ok: boolean; error: string | null } | 'checking' | null;

const input =
  'h-10 w-full min-w-0 rounded-lg border border-slate-200 bg-slate-50 px-3 text-sm text-slate-800 outline-none placeholder:text-slate-400 focus:border-indigo-500';
/** Quiet text action at the end of a pairing card. */
const cancelBtn = 'ml-auto -my-1.5 rounded px-2 py-1.5 text-xs text-slate-500 underline-offset-2 hover:text-slate-800 hover:underline';
const btn =
  'flex h-10 shrink-0 items-center justify-center gap-1.5 rounded-lg border border-slate-300 px-3.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-40';
const primary =
  'flex h-10 shrink-0 items-center justify-center gap-1.5 rounded-lg bg-indigo-500 px-3.5 text-sm font-semibold text-slate-50 hover:bg-indigo-400 disabled:opacity-40';
const card = 'rounded-xl border border-slate-200 bg-white';
const h2 = 'text-xl font-semibold text-slate-800';
const area =
  'w-full min-w-0 resize-none rounded-lg border border-slate-200 bg-slate-50 p-3 font-mono text-xs leading-relaxed text-slate-800 outline-none placeholder:text-slate-400 focus:border-indigo-500';
const choice = 'flex min-h-[76px] w-full items-center gap-3.5 rounded-xl border p-3.5 text-left';
const menuItem = 'flex h-11 w-full items-center rounded-lg px-3 text-left text-sm hover:bg-slate-100';

type View = 'choose' | 'join' | 'create';

/** A permission: title, one line of meaning, and a switch. The checkbox keeps its own label. */
function Permission({
  icon, title, detail, label, checked, onChange,
}: {
  icon: React.ReactNode;
  title: string;
  detail: string;
  label: string;
  checked: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3 rounded-lg bg-slate-100/50 p-3">
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-1.5 text-sm font-medium text-slate-800">
          {icon}
          {title}
        </span>
        <span className="text-xs text-slate-500">{detail}</span>
      </span>
      <input
        type="checkbox"
        aria-label={label}
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="peer sr-only"
      />
      <span
        aria-hidden="true"
        className="relative mt-0.5 h-5 w-9 shrink-0 rounded-full bg-slate-300 transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-4 after:w-4 after:rounded-full after:bg-slate-700 after:transition-transform peer-checked:bg-indigo-500 peer-checked:after:translate-x-4 peer-checked:after:bg-slate-50 peer-focus-visible:ring-2 peer-focus-visible:ring-indigo-300"
      />
    </label>
  );
}

const privateIcon = <Lock size={13} className="text-violet-300" />;
const agentsIcon = <SquareTerminal size={13} className="text-indigo-500" />;

function clock(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export function Network() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [net, setNet] = useState<NetworkView | null>(null);
  const [view, setView] = useState<View>('choose');
  const [name, setName] = useState('');
  const [address, setAddress] = useState('');
  const [relayKey, setRelayKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [checked, setChecked] = useState<Checked>(null);
  const [code, setCode] = useState('');
  const [relayCode, setRelayCode] = useState('');
  const [netName, setNetName] = useState('');
  const [creating, setCreating] = useState(false);
  const [menu, setMenu] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [rename, setRename] = useState('');
  const [showRelay, setShowRelay] = useState(false);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const nodes = useNodesStore((s) => s.nodes);
  const [mode, setMode] = useState<Mode>({ kind: 'idle' });
  const [pending, setPending] = useState<PairingPending | null>(null);
  const [left, setLeft] = useState(0);
  const [trustPrivate, setTrustPrivate] = useState(false);
  const [allowAgents, setAllowAgents] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const seen = useRef(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const closeMenu = useCallback(() => setMenu(false), []);
  useClickOutside(menuRef, menu, closeMenu);

  const reload = () => void useNodesStore.getState().load();
  const loadNet = () =>
    api.network().then(setNet).catch((e: unknown) => setError(errMsg(e)));
  const loadSettings = () =>
    api.settings().then((s) => {
      setSettings(s);
      setName(s.node_name ?? '');
      const r = splitRelay(s.relay_url);
      setAddress(r.address);
      setRelayKey(r.key);
    }).catch((e: unknown) => setError(errMsg(e)));

  useEffect(() => {
    void loadSettings();
    void loadNet();
    void useNodesStore.getState().load();
    // Settings may have been closed while pairing: show the running invite or join again.
    api.pairingState().then((st) => {
      if (st.invite && (st.stage === 'inviting' || st.stage === 'confirm')) {
        setLeft(st.invite.expires_at * 1000 - Date.now());
        setMode({ kind: 'invite', code: st.invite.code, expires: st.invite.expires_at });
      } else if (st.stage !== 'idle') {
        setMode({ kind: 'joining' });
        setView('join');
      }
    }).catch(() => {});
  }, []);

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const s = await api.relayStatus().catch(() => null);
      if (stopped) return;
      setStatus(s);
      timer = setTimeout(() => void poll(), 3000);
    };
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);

  const relayUrl = joinRelay(address, relayKey);

  const saveRelay = () => {
    if (!settings || relayUrl === (settings.relay_url ?? '')) return;
    setChecked(null);
    void save({ relay_url: relayUrl });
  };

  const check = async () => {
    setChecked('checking');
    try {
      const c = await api.relayCheck(relayUrl || undefined);
      setChecked({ ok: c.ok, error: c.error });
    } catch (e) {
      setChecked({ ok: false, error: errMsg(e) });
    }
  };
  const active = mode.kind === 'invite' || mode.kind === 'joining';
  const expires = mode.kind === 'invite' ? mode.expires : 0;

  useEffect(() => {
    if (!active) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    seen.current = false;
    const tick = async () => {
      if (expires) {
        const ms = expires * 1000 - Date.now();
        setLeft(ms);
        if (ms <= 0 && !seen.current) {
          setMode({ kind: 'idle' });
          return;
        }
      }
      const p = await api.pairingPending().catch(() => undefined);
      if (stopped) return;
      if (p) {
        seen.current = true;
        setPending(p);
      } else if (p === null && seen.current) {
        seen.current = false;
        setPending(null);
        setMode({ kind: 'idle' });
        void useNodesStore.getState().load();
        void loadNet();
        void loadSettings();
        return;
      }
      timer = setTimeout(() => void tick(), 1000);
    };
    timer = setTimeout(() => void tick(), 1000);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [active, expires]);

  const save = async (patch: { node_name?: string; relay_url?: string; network_name?: string }) => {
    try {
      setSettings(await api.patchSettings(patch));
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  const invite = async () => {
    try {
      const i = await api.pairingInvite();
      setLeft(i.expires_at * 1000 - Date.now());
      setMode({ kind: 'invite', code: i.code, expires: i.expires_at });
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  const join = async (c: string) => {
    try {
      await api.pairingJoin(c.trim(), settings?.node_name ?? '');
      setMode({ kind: 'joining' });
      setCode('');
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
  };

  const create = async () => {
    const url = relayFromCode(relayCode);
    setCreating(true);
    try {
      const c = await api.relayCheck(url);
      if (!c.ok) {
        setError(c.error ?? 'The relay did not answer');
        return;
      }
      setNet(await api.networkCreate(url, netName.trim()));
      setRelayCode('');
      setNetName('');
      setView('choose');
      setError(null);
      void loadSettings();
      reload();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setCreating(false);
    }
  };

  const leave = async () => {
    try {
      await api.networkLeave();
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
    setConfirmLeave(false);
    setShowRelay(false);
    setPending(null);
    setMode({ kind: 'idle' });
    setView('choose');
    void loadNet();
    void loadSettings();
    reload();
  };

  const saveName = async () => {
    setRenaming(false);
    if (rename.trim() === (net?.name ?? '')) return;
    await save({ network_name: rename });
    void loadNet();
  };

  const cancel = async () => {
    try {
      await api.pairingCancel();
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
    setPending(null);
    setMode({ kind: 'idle' });
  };

  const answer = async (ok: boolean) => {
    try {
      await (ok ? api.pairingConfirm(trustPrivate, allowAgents) : api.pairingReject());
    } catch (e) {
      setError(errMsg(e));
    }
    setPending(null);
    setTrustPrivate(false);
    setAllowAgents(false);
    setMode({ kind: 'idle' });
    reload();
    void loadNet();
  };

  const act = async (f: () => Promise<void>) => {
    try {
      await f();
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
    reload();
  };

  const self = nodes.find((n) => n.is_self);
  const selfName = settings?.node_name || name || self?.name || 'this device';
  const members = nodes.filter((n) => !n.is_self);
  const peer = pending?.peer_name || 'The new device';
  const errorLine = error && <span className="text-sm text-red-500">{error}</span>;

  const pairingCard = pending ? (
    <div className={`${card} flex flex-col gap-4 border-indigo-500 p-4`}>
      <div className="flex flex-col gap-1">
        <h3 className="font-semibold text-slate-800">Check and allow</h3>
        <p className="text-sm text-slate-500">Both screens must show the same number.</p>
      </div>
      <span className="text-center font-mono text-3xl tracking-[0.3em] text-slate-800">{pending.sas}</span>
      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold text-slate-500">What {peer} may do on {selfName}</span>
        {mode.kind === 'invite' && (
          <Permission
            icon={privateIcon}
            title="Private files"
            detail={`Syncs the private space with ${peer}`}
            label="Trust private"
            checked={trustPrivate}
            onChange={setTrustPrivate}
          />
        )}
        <Permission
          icon={agentsIcon}
          title="Run agents here"
          detail={`${peer} can start Claude and Antigravity on ${selfName}`}
          label="Allow agents"
          checked={allowAgents}
          onChange={setAllowAgents}
        />
      </div>
      {mode.kind === 'invite' && members.length > 0 && (
        <div className="flex flex-col gap-1.5 rounded-lg border border-slate-200 bg-slate-50 p-3 text-xs">
          <span className="font-semibold text-slate-600">{peer} will also meet</span>
          <ul aria-label={`${peer} will also meet`} className="flex flex-col gap-1">
            {members.map((n) => (
              <li key={n.node_id} className="flex justify-between gap-3 text-slate-500">
                <span className="truncate text-slate-700">{n.name}</span>
                {trustPrivate && n.trust === 'full' ? (
                  <span className="text-violet-300">private files</span>
                ) : (
                  <span>public files only</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="flex gap-2">
        <button aria-label="Reject" onClick={() => void answer(false)} className={`${btn} flex-1`}>
          Numbers differ
        </button>
        <button aria-label="Confirm" onClick={() => void answer(true)} className={`${primary} flex-1`}>
          Confirm
        </button>
      </div>
    </div>
  ) : mode.kind === 'invite' ? (
    <div className={`${card} flex flex-col gap-3 border-indigo-500 p-4`}>
      <div className="flex flex-col gap-1">
        <h3 className="font-semibold text-slate-800">Add a device to {net?.name ?? 'the network'}</h3>
        <p className="text-sm text-slate-500">
          On the new device open Settings, Network, Join a network and paste this code. It works once.
        </p>
      </div>
      <code className="select-all break-all rounded-lg bg-slate-50 p-3 font-mono text-xs text-slate-800">{mode.code}</code>
      <div className="flex items-center gap-3">
        <button
          aria-label="Copy code"
          onClick={() => void navigator.clipboard?.writeText(mode.code)}
          className={primary}
        >
          <Copy size={14} />
          Copy code
        </button>
        <span className="ml-auto text-xs text-slate-500">
          Expires in <span className="tabular-nums">{clock(left)}</span>
        </span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-slate-400">Waiting for the other device</span>
        <button onClick={() => void cancel()} className={cancelBtn}>
          Cancel
        </button>
      </div>
    </div>
  ) : mode.kind === 'joining' ? (
    <div className={`${card} flex items-center gap-3 p-4 text-sm text-slate-500`}>
      Connecting to the other device
      <button onClick={() => void cancel()} className={cancelBtn}>
        Cancel
      </button>
    </div>
  ) : null;

  if (!net) return <section aria-labelledby="settings-network" className="flex flex-col gap-4">{errorLine}</section>;

  if (!net.joined) {
    const back = (
      <button
        aria-label="Back"
        onClick={() => {
          setView('choose');
          setError(null);
        }}
        className="-ml-2 flex h-10 w-fit items-center gap-1 rounded-lg px-2 text-sm text-slate-500 hover:text-slate-800"
      >
        <ChevronLeft size={16} />
        Back
      </button>
    );
    if (view === 'join' || pairingCard) {
      const kind = classifyCode(code);
      return (
        <section aria-labelledby="settings-network" className="flex max-w-xl flex-col gap-4">
          {!pairingCard && back}
          <h2 id="settings-network" className={h2}>
            Join a network
          </h2>
          {pairingCard ?? (
            <>
              <ol className="flex flex-col gap-2.5 text-sm text-slate-600">
                <li className="flex gap-2.5">
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-slate-100 text-xs font-semibold text-slate-800">1</span>
                  On a device already in the network open Settings, Network, Add device.
                </li>
                <li className="flex gap-2.5">
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-slate-100 text-xs font-semibold text-slate-800">2</span>
                  Copy the code it shows and paste it here.
                </li>
              </ol>
              <textarea
                aria-label="Code from the other device"
                placeholder="Code from the other device"
                rows={3}
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className={area}
              />
              {kind === 'device' && <span className="text-sm text-slate-500">Joins the network on {codeHost(code)}</span>}
              {kind === 'invalid' && <span className="text-sm text-slate-500">This is not a code from Octo.</span>}
              {kind === 'relay' && (
                <span className="flex flex-wrap items-center gap-x-2 text-sm text-slate-500">
                  This is a relay code.
                  <button
                    onClick={() => {
                      setRelayCode(code);
                      setCode('');
                      setView('create');
                    }}
                    className="text-indigo-500 underline-offset-2 hover:underline"
                  >
                    Create a network with it
                  </button>
                </span>
              )}
              <button disabled={kind !== 'device'} onClick={() => void join(code)} className={primary}>
                Join
              </button>
            </>
          )}
          {errorLine}
        </section>
      );
    }
    if (view === 'create') {
      const ok = classifyCode(relayCode) === 'relay';
      return (
        <section aria-labelledby="settings-network" className="flex max-w-xl flex-col gap-4">
          {back}
          <div className="flex flex-col gap-1">
            <h2 id="settings-network" className={h2}>
              Create a network
            </h2>
            <p className="text-sm text-slate-500">
              Devices find each other through your relay. Its code is in the relay log after the first start.
            </p>
          </div>
          <textarea
            aria-label="Relay code"
            placeholder="relay.example.com?key=..."
            rows={3}
            value={relayCode}
            onChange={(e) => setRelayCode(e.target.value)}
            className={area}
          />
          <input
            aria-label="Network name"
            placeholder={codeHost(relayCode) || 'Network name'}
            value={netName}
            onChange={(e) => setNetName(e.target.value)}
            className={input}
          />
          <button disabled={!ok || creating} onClick={() => void create()} className={primary}>
            {creating ? 'Checking the relay...' : 'Create network'}
          </button>
          {errorLine}
          <p className="text-sm text-slate-500">
            Got a code from another device instead?{' '}
            <button onClick={() => setView('join')} className="text-indigo-500 underline-offset-2 hover:underline">
              Join a network
            </button>
          </p>
        </section>
      );
    }
    return (
      <section aria-labelledby="settings-network" className="flex max-w-xl flex-col gap-4">
        <div className="flex flex-col gap-1">
          <h2 id="settings-network" className={h2}>
            This device is not in a network
          </h2>
          <p className="text-sm text-slate-500">Connect it to your other devices to see their chats and files here.</p>
        </div>
        <button
          aria-label="Join a network"
          onClick={() => setView('join')}
          className={`${choice} border-indigo-500 bg-indigo-50 hover:bg-indigo-100`}
        >
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-indigo-500 text-slate-50">
            <LogIn size={19} />
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="font-semibold text-slate-800">Join a network</span>
            <span className="text-sm text-slate-600">Your other devices already use Octo</span>
          </span>
          <ChevronRight size={16} className="shrink-0 text-indigo-500" />
        </button>
        <button
          aria-label="Create a network"
          onClick={() => setView('create')}
          className={`${choice} border-slate-200 bg-white hover:bg-slate-100`}
        >
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-slate-100 text-slate-600">
            <Plus size={19} />
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="font-semibold text-slate-800">Create a network</span>
            <span className="text-sm text-slate-500">This is your first device and you have a relay</span>
          </span>
          <ChevronRight size={16} className="shrink-0 text-slate-400" />
        </button>
        {errorLine}
      </section>
    );
  }

  const devicesLabel = `${net.relay_host ?? 'No relay'} · ${net.devices} ${net.devices === 1 ? 'device' : 'devices'}`;
  return (
    <>
      <section aria-labelledby="settings-network" className="flex flex-col gap-4">
        <div className="flex items-center gap-3">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-indigo-50 text-indigo-500">
            <NetworkIcon size={19} />
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            {renaming ? (
              <input
                aria-label="Network name"
                autoFocus
                value={rename}
                placeholder={net.relay_host ?? 'Network name'}
                onChange={(e) => setRename(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void saveName();
                  if (e.key === 'Escape') setRenaming(false);
                }}
                onBlur={() => void saveName()}
                className={input}
              />
            ) : (
              <h2 id="settings-network" className={`${h2} truncate`}>
                {net.name}
              </h2>
            )}
            <span className="flex items-center gap-1.5 text-sm text-slate-500">
              {status?.active && (
                <span
                  role="img"
                  aria-label={status.online ? 'relay connected' : 'relay not connected'}
                  className={`h-1.5 w-1.5 shrink-0 rounded-full ${status.online ? 'bg-emerald-500' : 'bg-red-500'}`}
                />
              )}
              <span className="truncate">{devicesLabel}</span>
            </span>
          </div>
          <div
            ref={menuRef}
            className="relative"
            onKeyDown={(e) => {
              // Escape closes the menu only, not the whole Settings page.
              if (e.key === 'Escape' && menu) {
                e.stopPropagation();
                setMenu(false);
              }
            }}
          >
            <button
              aria-label="Network menu"
              aria-haspopup="menu"
              aria-expanded={menu}
              onClick={() => setMenu((v) => !v)}
              className="flex h-10 w-10 items-center justify-center rounded-lg border border-slate-200 bg-slate-100/50 text-slate-600 hover:bg-slate-100 max-md:h-11 max-md:w-11"
            >
              <MoreHorizontal size={18} />
            </button>
            {menu && (
              <div
                role="menu"
                aria-label="Network menu"
                className="absolute right-0 top-full z-20 mt-1 flex w-56 flex-col rounded-xl border border-slate-200 bg-white p-1.5 shadow-lg"
              >
                <button
                  role="menuitem"
                  onClick={() => {
                    setRename(net.name ?? '');
                    setRenaming(true);
                    setMenu(false);
                  }}
                  className={`${menuItem} text-slate-800`}
                >
                  Rename network
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    setShowRelay((v) => !v);
                    setMenu(false);
                  }}
                  className={`${menuItem} text-slate-800`}
                >
                  Relay
                </button>
                <div className="mx-1.5 my-1 h-px bg-slate-200" />
                <button
                  role="menuitem"
                  onClick={() => {
                    setConfirmLeave(true);
                    setMenu(false);
                  }}
                  className={`${menuItem} text-red-400`}
                >
                  Leave network
                </button>
              </div>
            )}
          </div>
        </div>

        {confirmLeave && (
          <div className={`${card} flex flex-col gap-3 border-red-400 p-4`}>
            <span className="text-sm text-slate-800">
              Leave {net.name}? This device stops seeing the other devices and their chats. They keep the network.
            </span>
            <div className="flex justify-end gap-2">
              <button onClick={() => setConfirmLeave(false)} className={btn}>
                Stay
              </button>
              <button
                onClick={() => void leave()}
                className="flex h-10 items-center rounded-lg bg-red-500 px-3.5 text-sm font-semibold text-white hover:bg-red-400"
              >
                Leave
              </button>
            </div>
          </div>
        )}

        {showRelay && settings && (
          <div className={`${card} flex flex-col gap-3 p-4`}>
            <label className="flex flex-col gap-1.5">
              <span className="text-xs font-semibold text-slate-500">Relay address</span>
              <div className="flex gap-2">
                <input
                  aria-label="Relay address"
                  placeholder="relay.example.com"
                  value={address}
                  onChange={(e) => {
                    setAddress(e.target.value);
                    setChecked(null);
                  }}
                  onBlur={saveRelay}
                  className={`${input} flex-1`}
                />
                <button
                  aria-label="Check relay"
                  disabled={!relayUrl || checked === 'checking'}
                  onClick={() => void check()}
                  className={btn}
                >
                  Check
                </button>
              </div>
            </label>
            <label className="flex flex-col gap-1.5">
              <span className="text-xs font-semibold text-slate-500">Key</span>
              <div className="flex gap-2">
                <input
                  aria-label="Relay key"
                  placeholder="Relay key"
                  type={showKey ? 'text' : 'password'}
                  autoComplete="off"
                  value={relayKey}
                  onChange={(e) => {
                    setRelayKey(e.target.value);
                    setChecked(null);
                  }}
                  onBlur={saveRelay}
                  className={`${input} flex-1`}
                />
                <button
                  aria-label={showKey ? 'Hide key' : 'Show key'}
                  onClick={() => setShowKey((v) => !v)}
                  className={btn}
                >
                  {showKey ? <EyeOff size={15} /> : <Eye size={15} />}
                </button>
              </div>
            </label>
            <RelayLine status={status} checked={checked} />
          </div>
        )}

        {settings && (
          <div className={`${card} flex items-center gap-3 p-3`}>
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-slate-100 text-slate-600">
              <Monitor size={18} />
            </span>
            <label className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="text-xs text-slate-500">This device, as others see it</span>
              <input
                aria-label="Node name"
                placeholder="Device name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                onBlur={() => name !== (settings.node_name ?? '') && void save({ node_name: name })}
                className={input}
              />
            </label>
          </div>
        )}

        {members.length > 0 && (
          <ul className={`${card} divide-y divide-slate-200`}>
            {members.map((n) => (
              <li key={n.node_id} className="flex flex-col gap-3 p-4">
                <div className="flex items-center gap-3">
                  <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-slate-100 text-slate-600">
                    <Monitor size={18} />
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="flex items-center gap-1.5 font-semibold text-slate-800">
                      <span className="truncate">{n.name}</span>
                      {n.trust === 'full' && <Lock data-testid="trust-lock" size={12} className="shrink-0 text-violet-300" />}
                    </span>
                    <span className="flex items-center gap-1.5 text-xs text-slate-500">
                      <span
                        role="img"
                        aria-label={n.online ? 'online' : 'offline'}
                        className={`h-1.5 w-1.5 rounded-full ${n.online ? 'bg-emerald-500' : 'bg-slate-300'}`}
                      />
                      {n.online ? 'Online' : 'Offline'}
                    </span>
                  </span>
                  <button
                    aria-label={`Remove ${n.name}`}
                    onClick={() => void act(() => api.removeNode(n.node_id))}
                    className="h-9 rounded-lg px-3 text-sm text-red-400 hover:bg-red-50 max-md:h-11"
                  >
                    Remove
                  </button>
                </div>
                <div className="grid grid-cols-2 gap-2 max-md:grid-cols-1">
                  <Permission
                    icon={privateIcon}
                    title="Private files"
                    detail={`Syncs the private space with ${n.name}`}
                    label={`Trust private ${n.name}`}
                    checked={n.trust === 'full'}
                    onChange={(on) => void act(() => api.setNodeTrust(n.node_id, on ? 'full' : 'public'))}
                  />
                  <Permission
                    icon={agentsIcon}
                    title="Run agents here"
                    detail={`${n.name} can start Claude and Antigravity on ${selfName}`}
                    label={`Allow agents ${n.name}`}
                    checked={n.agents === true}
                    onChange={(on) => void act(() => api.setNodeAgents(n.node_id, on))}
                  />
                </div>
              </li>
            ))}
          </ul>
        )}

        {pairingCard ?? (
          <button
            onClick={() => void invite()}
            className="flex h-12 items-center justify-center gap-2 rounded-xl border border-dashed border-slate-300 text-sm font-semibold text-slate-800 hover:bg-slate-100"
          >
            <Plus size={17} />
            Add device
          </button>
        )}
        {errorLine}
      </section>
    </>
  );
}

function RelayLine({ status, checked }: { status: RelayStatus | null; checked: Checked }) {
  let text = '';
  let tone: 'ok' | 'bad' | 'idle' = 'idle';
  if (checked === 'checking') {
    text = 'Checking...';
  } else if (checked) {
    text = checked.ok ? 'Relay OK' : (checked.error ?? 'Relay check failed');
    tone = checked.ok ? 'ok' : 'bad';
  } else if (status?.active) {
    text = status.online ? 'Connected' : (status.error ?? 'Connecting...');
    tone = status.online ? 'ok' : status.error ? 'bad' : 'idle';
  }
  if (!text) return null;
  const dot = tone === 'ok' ? 'bg-emerald-500' : tone === 'bad' ? 'bg-red-500' : 'bg-slate-300';
  return (
    <div role="status" className="flex items-center gap-2 text-sm text-slate-600">
      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dot}`} />
      <span className="min-w-0 break-words">{text}</span>
    </div>
  );
}
