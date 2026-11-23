import { useEffect, useRef, useState } from 'react';
import { useChatStore } from '../stores/chat';
import type { Message, PendingApproval } from '../api/types';
import { SquareTerminal } from 'lucide-react';
import { Markdown } from './Markdown';

const EMPTY: Message[] = [];
const NO_APPROVALS: PendingApproval[] = [];

function Bubble({ role, content }: { role: Message['role']; content: string }) {
  if (role === 'user') {
    return (
      <div className="ml-auto max-w-[85%] whitespace-pre-wrap rounded-2xl bg-slate-100 px-3 py-2 text-sm text-slate-800">
        {content}
      </div>
    );
  }
  if (role === 'error') {
    return <div className="whitespace-pre-wrap text-sm text-red-500">{content}</div>;
  }
  return <Markdown>{content}</Markdown>;
}

function ApprovalRow({ threadId, a }: { threadId: string; a: PendingApproval }) {
  const resolve = useChatStore((s) => s.resolveApproval);
  const decide = (allow: boolean) => void resolve(threadId, a.id, allow).catch(() => {});
  const [open, setOpen] = useState(false);
  const title = `Wants to use ${a.tool}`;
  return (
    <div
      role="group"
      aria-label={title}
      className="flex flex-col gap-2.5 rounded-xl border border-indigo-200 bg-indigo-50 p-3.5 text-sm"
    >
      <div className="flex items-center gap-2 font-semibold text-slate-800">
        <SquareTerminal size={16} className="text-indigo-500" />
        {title}
      </div>
      <button
        type="button"
        title="Show the full input"
        className="truncate rounded-lg bg-slate-50 px-2.5 py-2 text-left font-mono text-xs text-slate-700"
        onClick={() => setOpen((o) => !o)}
      >
        {a.summary}
      </button>
      {open && <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded-lg bg-slate-50 p-2.5 font-mono text-xs text-slate-700">{a.detail}</pre>}
      <div className="flex gap-2">
        <button
          type="button"
          className="h-8 rounded-lg bg-indigo-500 px-3.5 font-semibold text-slate-50 hover:bg-indigo-400 max-md:h-11 max-md:flex-1"
          onClick={() => decide(true)}
        >
          Allow
        </button>
        <button
          type="button"
          className="h-8 rounded-lg border border-slate-300 px-3.5 text-slate-700 hover:bg-slate-100 max-md:h-11 max-md:flex-1"
          onClick={() => decide(false)}
        >
          Deny
        </button>
      </div>
    </div>
  );
}

export function Feed({ threadId }: { threadId: string }) {
  const messages = useChatStore((s) => s.messages[threadId] ?? EMPTY);
  const streaming = useChatStore((s) => s.streaming[threadId]);
  const approvals = useChatStore((s) => s.approvals[threadId] ?? NO_APPROVALS);
  const endRef = useRef<HTMLDivElement>(null);
  const nearBottom = useRef(true);

  useEffect(() => {
    if (nearBottom.current) endRef.current?.scrollIntoView?.({ block: 'end' });
  }, [messages, streaming?.text]);

  return (
    <div
      className="flex-1 overflow-y-auto"
      onScroll={(e) => {
        const el = e.currentTarget;
        nearBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
      }}
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
        {messages.map((m) => (
          <Bubble key={m.id} role={m.role} content={m.content} />
        ))}
        <div aria-live="polite">
          {streaming &&
            (streaming.text ? (
              <Markdown>{streaming.text}</Markdown>
            ) : (
              <div className="text-xs text-slate-400">{streaming.tool ?? '...'}</div>
            ))}
        </div>
        {approvals.map((a) => (
          <ApprovalRow key={a.id} threadId={threadId} a={a} />
        ))}
        <div ref={endRef} />
      </div>
    </div>
  );
}
