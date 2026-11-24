import { useEffect, useState, type ReactNode } from 'react';
import { Globe, Lock, X } from 'lucide-react';
import { api } from '../api/client';
import { errMsg, relativeTime, shortCwd } from '../lib/format';
import { useThreadsStore } from '../stores/threads';
import { useNodesStore } from '../stores/nodes';
import type { ModelInfo, ProviderKind, ProviderStatus, SessionSummary, Space } from '../api/types';

const MODEL_PROVIDERS: ProviderKind[] = ['ollama', 'openrouter'];

const LABEL: Record<ProviderKind, string> = {
  claude: 'Claude Code',
  antigravity: 'Antigravity',
  ollama: 'Ollama',
  openrouter: 'OpenRouter',
};

function describe(p: ProviderStatus, node: string): string {
  if (!MODEL_PROVIDERS.includes(p.kind)) return `Agent · runs on ${node}`;
  return p.is_local ? 'Local model' : 'Cloud model';
}

function SearchList({
  label, query, onQuery, error, children,
}: {
  label: string;
  query: string;
  onQuery: (q: string) => void;
  error: string | null;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-slate-200 bg-slate-50 p-1.5">
      <input
        autoFocus
        type="search"
        aria-label={label}
        placeholder={label}
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        className="h-9 w-full rounded-md border border-slate-200 bg-white px-2.5 text-sm text-slate-800 outline-none placeholder:text-slate-400 focus:border-indigo-500"
      />
      {error && <div className="px-2 text-xs text-red-500">{error}</div>}
      <div role="listbox" aria-label={label} className="max-h-52 overflow-y-auto">
        {children}
      </div>
    </div>
  );
}

function ModelList({ kind, value, onPick }: { kind: ProviderKind; value: string | null; onPick: (id: string) => void }) {
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    let live = true;
    api.models(kind).then((m) => live && setModels(m)).catch((e: unknown) => live && setError(errMsg(e)));
    return () => {
      live = false;
    };
  }, [kind]);

  const q = query.trim().toLowerCase();
  const shown = (models ?? []).filter((m) => !q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q));
  return (
    <SearchList label="Search models" query={query} onQuery={setQuery} error={error}>
      {shown.map((m) => (
        <button
          key={m.id}
          role="option"
          aria-selected={m.id === value}
          onClick={() => onPick(m.id)}
          className={`block w-full truncate rounded-md px-2.5 py-2 text-left text-sm ${
            m.id === value ? 'bg-indigo-100 text-slate-800' : 'text-slate-700 hover:bg-slate-100'
          }`}
        >
          {m.name}
        </button>
      ))}
    </SearchList>
  );
}

function SessionList({
  nodeId, kind, onPick,
}: {
  nodeId: string | null;
  kind: ProviderKind;
  onPick: (externalId: string) => void;
}) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      api.importSessionsOn(nodeId, kind, query.trim())
        .then((s) => live && setSessions(s))
        .catch((e: unknown) => live && setError(errMsg(e)));
    }, query ? 150 : 0);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [nodeId, kind, query]);

  return (
    <SearchList label="Search sessions" query={query} onQuery={setQuery} error={error}>
      {sessions?.length === 0 && <div className="px-2.5 py-2 text-xs text-slate-400">No sessions found</div>}
      {(sessions ?? []).map((s) => (
        <button
          key={s.external_id}
          role="option"
          aria-selected={false}
          onClick={() => onPick(s.external_id)}
          className="block w-full rounded-md px-2.5 py-2 text-left hover:bg-slate-100"
        >
          <span className="block truncate text-sm text-slate-700">{s.title}</span>
          <span className="flex justify-between gap-2 text-xs text-slate-400">
            <span className="truncate">{shortCwd(s.cwd)}</span>
            <span className="shrink-0">{relativeTime(s.updated_at)}</span>
          </span>
        </button>
      ))}
    </SearchList>
  );
}

const sectionTitle = 'text-xs font-semibold text-slate-500';

function Choice({
  name, checked, disabled, onChange, title, detail, icon,
}: {
  name: string;
  checked: boolean;
  disabled?: boolean;
  onChange: () => void;
  title: string;
  detail: string;
  icon?: ReactNode;
}) {
  const id = `${name}-${title.replace(/\W+/g, '-')}`;
  return (
    <label
      className={`flex gap-2.5 rounded-xl border p-3 ${
        disabled
          ? 'cursor-not-allowed border-slate-200 opacity-50'
          : checked
            ? 'cursor-pointer border-indigo-500 bg-indigo-50'
            : 'cursor-pointer border-slate-200 bg-slate-100/40 hover:border-slate-300'
      }`}
    >
      <input
        type="radio"
        name={name}
        className="sr-only"
        checked={checked}
        disabled={disabled}
        onChange={onChange}
        aria-describedby={`${id}-d`}
      />
      {icon && <span className="mt-0.5 shrink-0 text-slate-500">{icon}</span>}
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-sm font-semibold text-slate-800">{title}</span>
        <span id={`${id}-d`} className="text-xs text-slate-500">
          {detail}
        </span>
      </span>
    </label>
  );
}

function Body({ onClose }: { onClose: () => void }) {
  const all = useThreadsStore((s) => s.providers);
  const nodes = useNodesStore((s) => s.nodes);
  const selectedId = useNodesStore((s) => s.selectedId);
  const self = nodes.find((n) => n.is_self);
  const openId = selectedId ?? self?.node_id ?? null;
  // The device the chat runs on: the open one unless another is picked here.
  const [picked, setDevice] = useState<string | null>(null);
  const device = picked ?? openId;
  const target = nodes.find((n) => n.node_id === device);
  const node = target?.name ?? 'this device';
  const elsewhere = device !== openId;
  // Assistants of a picked device; the open device's come from the store.
  const [fetched, setFetched] = useState<{ device: string | null; list: ProviderStatus[] } | null>(null);
  const remoteId = target?.is_self ? null : device;
  useEffect(() => {
    if (!elsewhere) return;
    let live = true;
    api.providersOn(remoteId)
      .then((list) => live && setFetched({ device, list }))
      .catch(() => live && setFetched({ device, list: [] }));
    return () => {
      live = false;
    };
  }, [device, elsewhere, remoteId]);
  const remote = elsewhere ? (fetched?.device === device ? fetched.list : []) : all;
  const providers = remote.filter((p) => p.installed);
  const [chosen, setKind] = useState<ProviderKind | null>(providers[0]?.kind ?? null);
  // A device may lack the chosen assistant: its first one is used instead.
  const kind = providers.some((p) => p.kind === chosen) ? chosen : (providers[0]?.kind ?? null);
  const [space, setSpace] = useState<Space>('public');
  const [ask, setAsk] = useState(true);
  const [model, setModel] = useState<string | null>(null);
  const [sessions, setSessions] = useState(false);

  const current = providers.find((p) => p.kind === kind);
  const needsModel = kind !== null && MODEL_PROVIDERS.includes(kind);
  const isAgent = kind !== null && !needsModel;
  const canPrivate = current?.is_local ?? false;
  const chosenSpace: Space = canPrivate ? space : 'public';

  const pick = (k: ProviderKind) => {
    setKind(k);
    setModel(null);
    setSessions(false);
  };

  /** Opens the picked device first, so the chat is created (and shown) there. */
  const goThere = () => {
    if (elsewhere) useNodesStore.getState().select(target?.is_self ? null : device);
  };

  const start = () => {
    if (!kind || (needsModel && !model)) return;
    onClose();
    goThere();
    void useThreadsStore.getState().create(kind, chosenSpace, ask ? 'confirm' : 'auto', model ?? undefined);
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="new-thread-title"
      className="relative flex max-h-[90vh] w-full max-w-xl flex-col gap-5 overflow-y-auto rounded-xl border border-slate-200 bg-white p-6 shadow-xl max-md:h-full max-md:max-h-none max-md:rounded-none max-md:border-0 max-md:p-4"
    >
      <div className="flex items-center justify-between">
        <h2 id="new-thread-title" className="text-lg font-semibold text-slate-800">
          New chat
        </h2>
        <button
          aria-label="Close"
          onClick={onClose}
          className="flex h-9 w-9 items-center justify-center rounded-lg text-slate-500 hover:bg-slate-100 hover:text-slate-800 max-md:h-11 max-md:w-11"
        >
          <X size={18} />
        </button>
      </div>

      {nodes.length > 1 && (
        <section className="flex flex-col gap-2">
          <h3 className={sectionTitle}>
            <label htmlFor="new-chat-device">Device</label>
          </h3>
          <select
            id="new-chat-device"
            aria-label="Device"
            value={device ?? ''}
            onChange={(e) => {
              setDevice(e.target.value);
              setModel(null);
              setSessions(false);
            }}
            className="h-11 rounded-lg border border-slate-300 bg-slate-50 px-3 text-sm text-slate-800 outline-none focus:border-indigo-500"
          >
            {nodes.map((n) => {
              const state = n.is_self ? 'this device' : n.online ? 'online' : 'offline';
              return (
                <option key={n.node_id} value={n.node_id} disabled={!n.is_self && !n.online}>
                  {`${n.name} · ${state}`}
                </option>
              );
            })}
          </select>
        </section>
      )}

      <section className="flex flex-col gap-2">
        <h3 className={sectionTitle}>Assistant</h3>
        {providers.length === 0 && <p className="text-sm text-slate-500">No assistants installed on {node}.</p>}
        <div role="radiogroup" aria-label="Assistant" className="grid grid-cols-2 gap-2 max-md:grid-cols-1">
          {providers.map((p) => (
            <Choice
              key={p.kind}
              name="assistant"
              checked={kind === p.kind}
              onChange={() => pick(p.kind)}
              title={LABEL[p.kind]}
              detail={describe(p, node)}
              icon={p.is_local ? <Lock size={13} className="text-violet-300" aria-hidden="true" /> : undefined}
            />
          ))}
        </div>
        {needsModel && kind && <ModelList key={kind} kind={kind} value={model} onPick={setModel} />}
      </section>

      <section className="flex flex-col gap-2">
        <h3 className={sectionTitle}>Space</h3>
        <div role="radiogroup" aria-label="Space" className="grid grid-cols-2 gap-2 max-md:grid-cols-1">
          <Choice
            name="space"
            checked={chosenSpace === 'private'}
            disabled={!canPrivate}
            onChange={() => setSpace('private')}
            title="Private"
            detail={canPrivate ? 'Stays on your devices' : 'Needs a local assistant'}
            icon={<Lock size={15} />}
          />
          <Choice
            name="space"
            checked={chosenSpace === 'public'}
            onChange={() => setSpace('public')}
            title="Public"
            detail="Public files, visible to paired devices"
            icon={<Globe size={15} />}
          />
        </div>
      </section>

      {isAgent && (
        <label className="flex cursor-pointer items-center gap-3 rounded-xl border border-slate-200 bg-slate-100/40 p-3">
          <span className="flex flex-1 flex-col gap-0.5">
            <span className="text-sm font-medium text-slate-800">Ask before running commands</span>
            <span className="text-xs text-slate-500">Off: the agent edits files and runs commands on its own</span>
          </span>
          <input
            type="checkbox"
            checked={ask}
            onChange={(e) => setAsk(e.target.checked)}
            className="h-4 w-4 accent-indigo-500"
          />
        </label>
      )}

      {isAgent && sessions && kind && (
        <SessionList
          key={`${device}-${kind}`}
          nodeId={target?.is_self ? null : device}
          kind={kind}
          onPick={(id) => {
            onClose();
            goThere();
            void useThreadsStore.getState().importSession(kind, id);
          }}
        />
      )}

      <div className="mt-auto flex items-center gap-3 max-md:flex-col-reverse max-md:items-stretch">
        {isAgent && current && (
          <button
            onClick={() => setSessions((v) => !v)}
            className="text-left text-sm text-indigo-500 hover:text-indigo-400 max-md:h-11 max-md:text-center"
          >
            Continue a {LABEL[current.kind]} session
          </button>
        )}
        <button
          onClick={start}
          disabled={!kind || (needsModel && !model)}
          className="ml-auto h-10 rounded-lg bg-indigo-500 px-5 text-sm font-semibold text-slate-50 hover:bg-indigo-400 disabled:opacity-40 max-md:ml-0 max-md:h-12"
        >
          {nodes.length > 1 ? `Start on ${node}` : 'Start'}
        </button>
      </div>
    </div>
  );
}

/** Choosing assistant, space and tool approval for a new thread, or continuing an existing session. */
export function NewThreadDialog() {
  const open = useThreadsStore((s) => s.menuOpen);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') useThreadsStore.getState().openMenu(false);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open]);
  if (!open) return null;
  const close = () => useThreadsStore.getState().openMenu(false);
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center p-4 max-md:p-0">
      <div className="absolute inset-0 bg-slate-900/40" onClick={close} />
      <Body onClose={close} />
    </div>
  );
}
