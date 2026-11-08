import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { api } from '../api/client';
import { Devices } from './Devices';
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
    <section className="flex flex-col gap-1.5">
      <h3 className="text-sm text-slate-800">{title}</h3>
      <div className="flex items-center gap-2">
        <input
          aria-label={label}
          type={type}
          value={value}
          placeholder={placeholder}
          autoComplete="off"
          onChange={(e) => onChange(e.target.value)}
          className="min-w-0 flex-1 rounded-lg border border-slate-200 bg-white px-2 py-1 text-sm text-slate-800 outline-none focus:border-indigo-500"
        />
        <button
          aria-label={`Check ${title}`}
          disabled={busy}
          onClick={() => void check()}
          className="rounded-lg px-2 py-1 text-xs text-slate-600 hover:bg-slate-100 disabled:opacity-40"
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

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [error, setError] = useState<string | null>(null);
  const downOnBackdrop = useRef(false);

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
      className="fixed inset-0 z-30 flex items-center justify-center bg-black/40"
      onMouseDown={(e) => (downOnBackdrop.current = e.target === e.currentTarget)}
      onMouseUp={(e) => {
        if (downOnBackdrop.current && e.target === e.currentTarget) onClose();
        downOnBackdrop.current = false;
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        className="flex max-h-[90vh] w-96 max-w-[90vw] flex-col overflow-y-auto gap-4 rounded-xl border border-slate-200 bg-white p-4 shadow-xl"
      >
        <button
          aria-label="Close"
          onClick={onClose}
          className="self-end rounded-md p-1 text-slate-400 hover:text-slate-700 max-md:p-3"
        >
          <X size={14} />
        </button>
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
            placeholder={settings?.openrouter_api_key ?? ''}
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
        <Devices />
      </div>
    </div>
  );
}
