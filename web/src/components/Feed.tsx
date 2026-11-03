import { useEffect, useRef } from 'react';
import { useChatStore } from '../stores/chat';
import type { Message } from '../api/types';
import { Markdown } from './Markdown';

const EMPTY: Message[] = [];

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

export function Feed({ threadId }: { threadId: string }) {
  const messages = useChatStore((s) => s.messages[threadId] ?? EMPTY);
  const streaming = useChatStore((s) => s.streaming[threadId]);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'end' });
  }, [messages, streaming?.text]);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
        {messages.map((m) => (
          <Bubble key={m.id} role={m.role} content={m.content} />
        ))}
        {streaming &&
          (streaming.text ? (
            <Markdown>{streaming.text}</Markdown>
          ) : (
            <div className="text-xs text-slate-400">{streaming.tool ?? '...'}</div>
          ))}
        <div ref={endRef} />
      </div>
    </div>
  );
}
