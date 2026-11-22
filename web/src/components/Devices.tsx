import { useEffect, useRef, useState } from 'react';
import { Check, Copy, Eye, EyeOff, Lock, Terminal, X } from 'lucide-react';
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
  'min-w-0 flex-1 rounded-lg border border-slate-200 bg-white px-2 py-1 text-sm text-slate-800 outline-none focus:border-indigo-500';
const btn = 'rounded-lg px-2 py-1 text-xs text-slate-600 hover:bg-slate-100 disabled:opacity-40';

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

  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-sm text-slate-800">Devices</h3>
      {settings && (
        <>
          <input
            aria-label="Node name"
            placeholder="Device name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => name !== (settings.node_name ?? '') && void save({ node_name: name })}
            className={input}
          />
          <div className="flex items-center gap-2">
            <input
              aria-label="Relay address"
              placeholder="relay.example.com"
              value={address}
              onChange={(e) => {
                setAddress(e.target.value);
                setChecked(null);
              }}
              onBlur={saveRelay}
              className={input}
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
          <div className="flex items-center gap-2">
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
              className={input}
            />
            <button
              aria-label={showKey ? 'Hide key' : 'Show key'}
              onClick={() => setShowKey((v) => !v)}
              className={btn}
            >
              {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
          <RelayLine status={status} checked={checked} />
        </>
      )}
      <ul className="flex flex-col">
        {nodes.filter((n) => !n.is_self).map((n) => (
          <li key={n.node_id} className="flex items-center gap-2 py-0.5 text-sm text-slate-700">
            <span
              role="img"
              aria-label={n.online ? 'online' : 'offline'}
              className={`h-1.5 w-1.5 rounded-full ${n.online ? 'bg-emerald-500' : 'bg-slate-300'}`}
            />
            {n.name}
            {n.trust === 'full' && <Lock data-testid="trust-lock" size={12} className="text-slate-500" />}
            <label title="Private data" className="ml-auto flex max-md:p-3">
              <input
                type="checkbox"
                aria-label={`Trust private ${n.name}`}
                checked={n.trust === 'full'}
                onChange={(e) => void act(() => api.setNodeTrust(n.node_id, e.target.checked ? 'full' : 'public'))}
              />
            </label>
            <label title="Agents" className="flex max-md:p-3">
              <input
                type="checkbox"
                aria-label={`Allow agents ${n.name}`}
                checked={n.agents === true}
                onChange={(e) => void act(() => api.setNodeAgents(n.node_id, e.target.checked))}
              />
              <Terminal size={12} className="ml-1 text-slate-500" />
            </label>
            <button
              aria-label={`Remove ${n.name}`}
              onClick={() => void act(() => api.removeNode(n.node_id))}
              className={`${btn} max-md:p-3`}
            >
              <X size={12} />
            </button>
          </li>
        ))}
      </ul>
      {pending ? (
        <div className="flex items-center gap-2">
          <span className="font-mono text-lg tracking-widest text-slate-800">{pending.sas}</span>
          {mode.kind === 'invite' && (
            <label className="flex items-center gap-1 text-xs text-slate-600">
              <input
                type="checkbox"
                aria-label="Trust private"
                checked={trustPrivate}
                onChange={(e) => setTrustPrivate(e.target.checked)}
              />
              <Lock size={12} />
            </label>
          )}
          <label className="flex items-center gap-1 text-xs text-slate-600">
            <input
              type="checkbox"
              aria-label="Allow agents"
              checked={allowAgents}
              onChange={(e) => setAllowAgents(e.target.checked)}
            />
            <Terminal size={12} />
          </label>
          <button aria-label="Confirm" onClick={() => void answer(true)} className={btn}>
            <Check size={14} />
          </button>
          <button aria-label="Reject" onClick={() => void answer(false)} className={btn}>
            <X size={14} />
          </button>
        </div>
      ) : mode.kind === 'invite' ? (
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 select-all break-all text-xs text-slate-800">{mode.code}</code>
          <button
            aria-label="Copy code"
            onClick={() => void navigator.clipboard?.writeText(mode.code)}
            className={btn}
          >
            <Copy size={14} />
          </button>
          <span className="text-xs tabular-nums text-slate-500">{clock(left)}</span>
        </div>
      ) : mode.kind === 'idle' ? (
        <>
          <div className="flex items-center gap-2">
            <input
              aria-label="Code"
              placeholder="Code from the other device"
              value={code}
              onChange={(e) => setCode(e.target.value)}
              className={input}
            />
            <button disabled={!code.trim()} onClick={() => void join(code)} className={btn}>
              Join
            </button>
          </div>
          <div className="flex gap-2">
            <button onClick={() => void invite()} className={btn}>
              Add device
            </button>
          </div>
        </>
      ) : null}
      {error && <span className="text-xs text-red-500">{error}</span>}
    </section>
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
    <div role="status" className="flex items-center gap-2 px-1 text-xs text-slate-600">
      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dot}`} />
      <span className="min-w-0 break-words">{text}</span>
    </div>
  );
}
