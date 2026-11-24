import { useEffect, useState } from 'react';
import { ChevronLeft } from 'lucide-react';
import { api } from '../api/client';
import { Network } from './Network';
import { errMsg } from '../lib/format';
import type { ProviderKind, SettingsView, SettingsPatch } from '../api/types';

type Result = { ok: true; count: number } | { ok: false; error: string };

function Section({
  title, label, type, value, placeholder, onChange, onCheck,
}: {
  title: string;
  label: string;
  type: 'text' | 'password';
  value: string;
  placeholder?: string;
  onChange: (v: string) => void;
  onCheck: () => Promise<Result>;
}) {
  const [result, setResult] = useState<Result | null>(null);
  const [busy, setBusy] = useState(false);
  const check = async () => {
    setBusy(true);
    setResult(await onCheck());
    setBusy(false);
  };
  return (
    <section className="flex flex-col gap-2 rounded-xl border border-slate-200 bg-white p-4">
      <h3 className="text-sm font-semibold text-slate-800">{title}</h3>
      <div className="flex items-center gap-2">
        <input
          aria-label={label}
          type={type}
          value={value}
          placeholder={placeholder}
          autoComplete="off"
          onChange={(e) => onChange(e.target.value)}
          className="h-10 min-w-0 flex-1 rounded-lg border border-slate-200 bg-slate-50 px-3 text-sm text-slate-800 outline-none placeholder:text-slate-400 focus:border-indigo-500"
        />
        <button
          aria-label={`Check ${title}`}
          disabled={busy}
          onClick={() => void check()}
          className="h-10 shrink-0 rounded-lg border border-slate-300 px-3.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-40"
        >
          Check
        </button>
      </div>
      {result && (
        <span className={`text-xs ${result.ok ? 'text-slate-500' : 'text-red-500'}`}>
          {result.ok ? `${result.count} models` : result.error}
        </span>
      )}
    </section>
  );
}

const SECTIONS = [
  { id: 'settings-models', label: 'Models' },
  { id: 'settings-network', label: 'Network' },
];

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.settings().then((s) => {
      setSettings(s);
      setUrl(s.ollama_url);
    }).catch((e: unknown) => setError(errMsg(e)));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const run = async (patch: SettingsPatch, kind: ProviderKind): Promise<Result> => {
    try {
      if (Object.keys(patch).length) setSettings(await api.patchSettings(patch));
      return { ok: true, count: (await api.models(kind)).length };
    } catch (e) {
      return { ok: false, error: errMsg(e) };
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      className="fixed inset-0 z-30 flex bg-slate-50 text-slate-800 max-md:flex-col"
    >
      <nav
        aria-label="Settings sections"
        className="flex w-60 shrink-0 flex-col gap-0.5 border-r border-slate-200 bg-white p-3 max-md:w-full max-md:flex-row max-md:items-center max-md:gap-1 max-md:overflow-x-auto max-md:border-b max-md:border-r-0 max-md:p-2"
      >
        <button
          aria-label="Close"
          onClick={onClose}
          className="mb-3 flex h-9 items-center gap-1.5 rounded-lg px-2 text-sm text-slate-600 hover:bg-slate-100 hover:text-slate-800 max-md:mb-0 max-md:h-11 max-md:shrink-0"
        >
          <ChevronLeft size={16} />
          <span className="max-md:hidden">Back to chats</span>
        </button>
        <span className="px-2 pb-2 text-lg font-semibold max-md:hidden">Settings</span>
        {SECTIONS.map((x) => (
          <a
            key={x.id}
            href={`#${x.id}`}
            onClick={(e) => {
              e.preventDefault();
              document.getElementById(x.id)?.scrollIntoView?.({ behavior: 'smooth', block: 'start' });
            }}
            className="flex h-9 items-center rounded-lg px-2.5 text-sm text-slate-600 hover:bg-slate-100 hover:text-slate-800 max-md:h-11 max-md:shrink-0"
          >
            {x.label}
          </a>
        ))}
      </nav>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-10 px-8 py-8 max-md:px-4 max-md:py-5">
          <section aria-labelledby="settings-models" className="flex flex-col gap-4">
            <div className="flex flex-col gap-1">
              <h2 id="settings-models" className="text-xl font-semibold text-slate-800">
                Models
              </h2>
              <p className="text-sm text-slate-500">Local models through Ollama, cloud models through OpenRouter.</p>
            </div>
            {settings && (
              <>
                <Section
                  title="Ollama"
                  label="Ollama URL"
                  type="text"
                  value={url}
                  onChange={setUrl}
                  onCheck={() => run(url !== settings?.ollama_url ? { ollama_url: url } : {}, 'ollama')}
                />
                <Section
                  title="OpenRouter"
                  label="OpenRouter key"
                  type="password"
                  value={key}
                  placeholder={settings?.openrouter_api_key ?? 'OpenRouter API key'}
                  onChange={setKey}
                  onCheck={async () => {
                    const r = await run(key ? { openrouter_api_key: key } : {}, 'openrouter');
                    if (r.ok) setKey('');
                    return r;
                  }}
                />
              </>
            )}
            {error && <span className="text-xs text-red-500">{error}</span>}
          </section>
          <Network />
        </div>
      </div>
    </div>
  );
}
