import React, { useEffect, useRef, useState } from 'react';
import { Copy, Eye, EyeOff, Lock, Monitor, SquareTerminal } from 'lucide-react';
import { api } from '../api/client';
import { useNodesStore } from '../stores/nodes';
import { errMsg } from '../lib/format';
import { joinRelay, splitRelay } from '../lib/relay';
import type { PairingPending, RelayStatus, SettingsView } from '../api/types';

type Mode =
  | { kind: 'idle' }
  | { kind: 'invite'; code: string; expires: number }
  | { kind: 'joining' };

/** Result of the last "Check", shown until the relay fields change. */
type Checked = { ok: boolean; error: string | null } | 'checking' | null;

const input =
  'h-10 w-full min-w-0 rounded-lg border border-slate-200 bg-slate-50 px-3 text-sm text-slate-800 outline-none placeholder:text-slate-400 focus:border-indigo-500';
const btn =
  'flex h-10 shrink-0 items-center justify-center gap-1.5 rounded-lg border border-slate-300 px-3.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-40';
const primary =
  'flex h-10 shrink-0 items-center justify-center gap-1.5 rounded-lg bg-indigo-500 px-3.5 text-sm font-semibold text-slate-50 hover:bg-indigo-400 disabled:opacity-40';
const card = 'rounded-xl border border-slate-200 bg-white';
const h2 = 'text-xl font-semibold text-slate-800';

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

export function Devices() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [name, setName] = useState('');
  const [address, setAddress] = useState('');
  const [relayKey, setRelayKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [status, setStatus] = useState<RelayStatus | null>(null);
  const [checked, setChecked] = useState<Checked>(null);
  const [code, setCode] = useState('');
  const nodes = useNodesStore((s) => s.nodes);
  const [mode, setMode] = useState<Mode>({ kind: 'idle' });
  const [pending, setPending] = useState<PairingPending | null>(null);
  const [left, setLeft] = useState(0);
  const [trustPrivate, setTrustPrivate] = useState(false);
  const [allowAgents, setAllowAgents] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const seen = useRef(false);

  const reload = () => void useNodesStore.getState().load();

  useEffect(() => {
    api.settings().then((s) => {
      setSettings(s);
      setName(s.node_name ?? '');
      const r = splitRelay(s.relay_url);
      setAddress(r.address);
      setRelayKey(r.key);
    }).catch((e: unknown) => setError(errMsg(e)));
    void useNodesStore.getState().load();
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

  const save = async (patch: { node_name?: string; relay_url?: string }) => {
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

  const join = async (code: string) => {
    try {
      await api.pairingJoin(code.trim(), settings?.node_name || name || 'Octo');
      setMode({ kind: 'joining' });
      setCode('');
      setError(null);
    } catch (e) {
      setError(errMsg(e));
    }
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

  return (
    <>
      <section aria-labelledby="settings-devices" className="flex flex-col gap-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <h2 id="settings-devices" className={h2}>
              Devices
            </h2>
            <p className="text-sm text-slate-500">
              What other devices may do on <b className="font-semibold text-slate-800">{selfName}</b>. Each device sets this for itself.
            </p>
          </div>
          {mode.kind === 'idle' && !pending && (
            <button onClick={() => void invite()} className={primary}>
              Add device
            </button>
          )}
        </div>

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

        {pending ? (
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
              <h3 className="font-semibold text-slate-800">Share this code</h3>
              <p className="text-sm text-slate-500">
                On the new device: Settings, Devices, paste the code and Join. It works once and already contains the relay address and key.
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
            <span className="text-xs text-slate-400">Waiting for the other device</span>
          </div>
        ) : mode.kind === 'joining' ? (
          <div className={`${card} p-4 text-sm text-slate-500`}>Connecting to the other device</div>
        ) : (
          <div className={`${card} flex flex-col gap-2 p-4`}>
            <span className="text-sm font-medium text-slate-800">Join with a code</span>
            <span className="text-xs text-slate-500">Paste the code that another device shows under Add device.</span>
            <div className="flex gap-2 max-md:flex-col">
              <input
                aria-label="Code"
                placeholder="Code from the other device"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className={`${input} flex-1 max-md:flex-none`}
              />
              <button disabled={!code.trim()} onClick={() => void join(code)} className={btn}>
                Join
              </button>
            </div>
          </div>
        )}
        {error && <span className="text-sm text-red-500">{error}</span>}
      </section>

      {settings && (
        <section aria-labelledby="settings-relay" className="flex flex-col gap-4">
          <div className="flex flex-col gap-1">
            <h2 id="settings-relay" className={h2}>
              Relay
            </h2>
            <p className="text-sm text-slate-500">Devices reach each other through the relay. Its address and key come with a code.</p>
          </div>
          <div className={`${card} flex flex-col gap-3 p-4`}>
            <label className="flex flex-col gap-1.5">
              <span className="text-xs font-semibold text-slate-500">Address</span>
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
        </section>
      )}
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
