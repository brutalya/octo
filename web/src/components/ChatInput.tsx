import { useCallback, useRef, useState, type DragEvent, type KeyboardEvent } from 'react';
import { ArrowUp, Paperclip, Square, X } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { DRAG_TYPE } from './FileTree';
import { useClickOutside } from '../lib/useClickOutside';
import type { AttachmentRef, Space } from '../api/types';

interface Props {
  space?: Space;
  streaming: boolean;
  disabled?: boolean;
  /** Resolve false when the message was rejected before being appended. */
  onSend: (text: string, attachments: AttachmentRef[]) => unknown;
  onStop: () => void;
}

const EMPTY: never[] = [];

function Picker({ space, picked, onPick }: { space: Space; picked: AttachmentRef[]; onPick: (path: string) => void }) {
  const entries = useFilesStore((s) => s.entries[space] ?? EMPTY);
  const files = entries.filter((e) => e.kind === 'file' && !picked.some((a) => a.path === e.path));
  return (
    <div
      role="menu"
      className="absolute bottom-full left-0 z-10 mb-1 max-h-60 min-w-48 overflow-y-auto rounded-lg border border-slate-200 bg-white p-1 shadow-lg"
    >
      {files.map((f) => (
        <button
          key={f.path}
          role="menuitem"
          onClick={() => onPick(f.path)}
          className="block w-full truncate rounded px-2 py-1 text-left text-sm text-slate-700 hover:bg-slate-100"
        >
          {f.path}
        </button>
      ))}
    </div>
  );
}

export function ChatInput({ space, streaming, disabled, onSend, onStop }: Props) {
  const [value, setValue] = useState('');
  const [attachments, setAttachments] = useState<AttachmentRef[]>([]);
  const [picking, setPicking] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const closePicker = useCallback(() => setPicking(false), []);
  useClickOutside(ref, picking, closePicker);

  const add = (a: AttachmentRef) =>
    setAttachments((cur) => (cur.some((x) => x.space === a.space && x.path === a.path) ? cur : [...cur, a]));

  const submit = () => {
    const text = value.trim();
    if (!text || streaming || disabled) return;
    const sent = attachments;
    setValue('');
    setAttachments([]);
    void Promise.resolve(onSend(text, sent)).then((ok) => {
      if (ok !== false) return;
      setValue((cur) => cur || text);
      setAttachments((cur) => (cur.length ? cur : sent));
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const onDragOver = (e: DragEvent) => {
    if (e.dataTransfer.types?.includes(DRAG_TYPE)) e.preventDefault();
  };

  const onDrop = (e: DragEvent) => {
    const raw = e.dataTransfer.getData(DRAG_TYPE);
    if (!raw) return;
    e.preventDefault();
    try {
      const a = JSON.parse(raw) as AttachmentRef;
      if (space && a.space === space && typeof a.path === 'string') add({ space: a.space, path: a.path });
    } catch {
      // not ours
    }
  };

  return (
    <div
      data-testid="chat-input"
      onDragOver={onDragOver}
      onDrop={onDrop}
      className="mx-auto flex w-full max-w-3xl flex-col gap-2 px-4 pb-4 max-md:px-2 max-md:pb-[max(0.5rem,env(safe-area-inset-bottom))]"
    >
      {attachments.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {attachments.map((a) => (
            <span
              key={`${a.space}:${a.path}`}
              className="flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-xs text-slate-600"
            >
              <span title={a.path}>{a.path.split('/').pop() || a.path}</span>
              <button
                aria-label={`Remove ${a.path}`}
                onClick={() => setAttachments((cur) => cur.filter((x) => x !== a))}
                className="text-slate-400 hover:text-slate-700 max-md:p-3"
              >
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2">
        {space && (
          <div ref={ref} className="relative">
            <button
              aria-label="Attach"
              onClick={() => {
                if (!picking) void useFilesStore.getState().load(space);
                setPicking((v) => !v);
              }}
              className="rounded-xl p-2.5 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
            >
              <Paperclip size={16} />
            </button>
            {picking && (
              <Picker
                space={space}
                picked={attachments}
                onPick={(path) => {
                  add({ space, path });
                  setPicking(false);
                }}
              />
            )}
          </div>
        )}
        <textarea
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={onKeyDown}
          rows={1}
          className="max-h-48 min-h-10 flex-1 resize-none rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm text-slate-800 outline-none focus:border-indigo-500"
        />
        {streaming ? (
          <button
            aria-label="Stop"
            onClick={onStop}
            className="rounded-xl bg-slate-100 p-2.5 text-slate-700 hover:bg-slate-200"
          >
            <Square size={16} />
          </button>
        ) : (
          <button
            aria-label="Send"
            disabled={disabled}
            onClick={submit}
            className="rounded-xl bg-indigo-500 p-2.5 text-slate-50 hover:bg-indigo-600 disabled:opacity-40"
          >
            <ArrowUp size={16} />
          </button>
        )}
      </div>
    </div>
  );
}
