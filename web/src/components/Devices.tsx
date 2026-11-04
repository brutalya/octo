import { useCallback, useEffect, useState } from 'react';
import { Check, Lock, Plus, X } from 'lucide-react';
import { api } from '../api/client';
import type { NodeInfo, PairingPending, SettingsView } from '../api/types';

type Mode =
  | { kind: 'idle' }
  | { kind: 'invite'; code: string; expires: number }
  | { kind: 'join'; code: string }
  | { kind: 'joining' };

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
  const [relay, setRelay] = useState('');
  const [nodes, setNodes] = useState<NodeInfo[]>([]);
  const [mode, setMode] = useState<Mode>({ kind: 'idle' });
  const [pending, setPending] = useState<PairingPending | null>(null);
  const [left, setLeft] = useState(0);
  const [trustPrivate, setTrustPrivate] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    api.nodes().then((n) => Array.isArray(n) && setNodes(n)).catch(() => {});
  }, []);

  useEffect(() => {
    api.settings().then((s) => {
      setSettings(s);
      setName(s.node_name ?? '');
      setRelay(s.relay_url ?? '');
    }).catch(() => {});
    reload();
  }, [reload]);

  const active = mode.kind === 'invite' || mode.kind === 'joining';
  const expires = mode.kind === 'invite' ? mode.expires : 0;

  useEffect(() => {
    if (!active) return;
    let seen = false;
    const tick = setInterval(() => {
      if (expires) {
        const ms = expires * 1000 - Date.now();
        setLeft(ms);
        if (ms <= 0 && !seen) {
          setMode({ kind: 'idle' });
          return;
        }
      }
      api.pairingPending().then((p) => {
        if (p) {
          seen = true;
          setPending(p);
        } else if (seen) {
          seen = false;
          setPending(null);
          setMode({ kind: 'idle' });
          reload();
        }
      }).catch(() => {});
    }, 1000);
    return () => clearInterval(tick);
  }, [active, expires, reload]);

  const save = async (patch: { node_name?: string; relay_url?: string }) => {
    try {
      setSettings(await api.patchSettings(patch));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const invite = async () => {
    try {
      const i = await api.pairingInvite();
      setLeft(i.expires_at * 1000 - Date.now());
      setMode({ kind: 'invite', code: i.code, expires: i.expires_at });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const join = async (code: string) => {
    try {
      await api.pairingJoin(code.trim(), settings?.node_name || name || 'Octo');
      setMode({ kind: 'joining' });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const answer = async (ok: boolean) => {
    try {
      await (ok ? api.pairingConfirm(trustPrivate) : api.pairingReject());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setPending(null);
    setTrustPrivate(false);
    setMode({ kind: 'idle' });
    reload();
  };

  const act = async (f: () => Promise<void>) => {
    try {
      await f();
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    reload();
  };

  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-sm text-slate-800">Devices</h3>
      <input
        aria-label="Node name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onBlur={() => name !== (settings?.node_name ?? '') && void save({ node_name: name })}
        className={input}
      />
      <input
        aria-label="Relay URL"
        value={relay}
        onChange={(e) => setRelay(e.target.value)}
        onBlur={() => relay !== (settings?.relay_url ?? '') && void save({ relay_url: relay })}
        className={input}
      />
      <ul className="flex flex-col">
        {nodes.filter((n) => !n.is_self).map((n) => (
          <li key={n.node_id} className="flex items-center gap-2 py-0.5 text-sm text-slate-700">
            <span className={`h-1.5 w-1.5 rounded-full ${n.online ? 'bg-emerald-500' : 'bg-slate-300'}`} />
            {n.name}
            {n.trust === 'full' && <Lock data-testid="trust-lock" size={12} className="text-slate-500" />}
            <input
              type="checkbox"
              aria-label={`Trust private ${n.name}`}
              checked={n.trust === 'full'}
              onChange={(e) => void act(() => api.setNodeTrust(n.node_id, e.target.checked ? 'full' : 'public'))}
              className="ml-auto"
            />
            <button aria-label={`Remove ${n.name}`} onClick={() => void act(() => api.removeNode(n.node_id))} className={btn}>
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
          <span className="text-xs tabular-nums text-slate-500">{clock(left)}</span>
        </div>
      ) : mode.kind === 'join' ? (
        <div className="flex items-center gap-2">
          <input
            aria-label="Code"
            autoFocus
            value={mode.code}
            onChange={(e) => setMode({ kind: 'join', code: e.target.value })}
            className={input}
          />
          <button disabled={!mode.code.trim()} onClick={() => void join(mode.code)} className={btn}>
            Connect
          </button>
        </div>
      ) : mode.kind === 'idle' ? (
        <div className="flex gap-2">
          <button aria-label="Add device" onClick={() => void invite()} className={btn}>
            <Plus size={14} />
          </button>
          <button onClick={() => setMode({ kind: 'join', code: '' })} className={btn}>
            Join
          </button>
        </div>
      ) : null}
      {error && <span className="text-xs text-red-500">{error}</span>}
    </section>
  );
}
