import { useEffect, useRef } from 'react';
import { useChatStore } from '../stores/chat';
import type { Message, PendingApproval } from '../api/types';
import { Check, X } from 'lucide-react';
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
  return (
    <div className="flex items-center gap-2 text-sm text-slate-600">
      <span className="min-w-0 flex-1 truncate font-mono text-xs">{a.summary}</span>
      <button type="button" aria-label="Allow" className="rounded p-1 hover:bg-slate-100" onClick={() => decide(true)}>
        <Check size={16} />
      </button>
      <button type="button" aria-label="Deny" className="rounded p-1 hover:bg-slate-100" onClick={() => decide(false)}>
        <X size={16} />
      </button>
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
