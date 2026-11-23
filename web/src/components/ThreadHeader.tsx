import { Globe, Lock } from 'lucide-react';
import { useThreadsStore } from '../stores/threads';
import { shortCwd } from '../lib/format';
import type { Approval, Thread } from '../api/types';

const MODES: { value: Approval; label: string }[] = [
  { value: 'confirm', label: 'Ask before tools' },
  { value: 'auto', label: 'Auto' },
];

/** Desktop bar above the chat: what this thread is and how its tools run. */
export function ThreadHeader({ thread }: { thread: Thread }) {
  const setApproval = useThreadsStore((s) => s.setApproval);
  const detail = [thread.model ?? thread.provider, shortCwd(thread.cwd)].filter(Boolean).join(' · ');
  const isPrivate = thread.space === 'private';
  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-slate-200 px-5">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className={`truncate text-[15px] font-semibold ${thread.title ? 'text-slate-800' : 'italic text-slate-400'}`}>
          {thread.title || 'Untitled'}
        </span>
        <span className="truncate text-xs text-slate-400">{detail}</span>
      </div>
      <span
        className={`flex h-7 items-center gap-1.5 rounded-full border border-slate-200 bg-slate-100/50 px-2.5 text-xs ${
          isPrivate ? 'text-violet-300' : 'text-slate-600'
        }`}
      >
        {isPrivate ? <Lock size={12} /> : <Globe size={12} />}
        {isPrivate ? 'Private' : 'Public'}
      </span>
      <div role="group" aria-label="Tool approval" className="flex rounded-lg border border-slate-200 bg-slate-100/50 p-0.5">
        {MODES.map((m) => (
          <button
            key={m.value}
            aria-pressed={thread.approval === m.value}
            onClick={() => thread.approval !== m.value && void setApproval(thread.id, m.value)}
            className={`h-7 rounded-md px-2.5 text-xs ${
              thread.approval === m.value ? 'bg-slate-200 text-slate-800' : 'text-slate-500 hover:text-slate-800'
            }`}
          >
            {m.label}
          </button>
        ))}
      </div>
    </header>
  );
}
