import { useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent } from 'react';
import { ArrowUp, Folder, Paperclip, Square, X } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { DRAG_TYPE } from './FileTree';
import { useAttachStore } from '../stores/attach';
import { attachDir, pastedName, uniqueName } from '../lib/files';
import type { AttachmentRef, Space } from '../api/types';

interface Props {
  space?: Space;
  /** Title of the chat: files sent from here are kept in a folder named after it. */
  title?: string | null;
  streaming: boolean;
  disabled?: boolean;
  placeholder?: string;
  /** Resolve false when the message was rejected before being appended. */
  onSend: (text: string, attachments: AttachmentRef[]) => unknown;
  onStop: () => void;
}

export function ChatInput({ space, title, streaming, disabled, placeholder, onSend, onStop }: Props) {
  const [value, setValue] = useState('');
  const [attachments, setAttachments] = useState<AttachmentRef[]>([]);
  const [showPicked, setShowPicked] = useState(false);
  const fromFiles = useAttachStore((s) => s.fromFiles);
  const fileInput = useRef<HTMLInputElement>(null);
  // Picked in Files for a chat of this space (another space's picks wait for their own chat).
  const picked = fromFiles.filter((p) => p.space === space);

  const [uploading, setUploading] = useState<string[]>([]);
  const [uploadError, setUploadError] = useState<string | null>(null);

  /** Uploads to the chat's attachments folder under names that do not overwrite anything, then attaches. */
  const uploadAndAttach = async (files: File[]) => {
    if (!space || !files.length) return;
    const store = useFilesStore.getState();
    const dir = attachDir(title);
    const taken = new Set(
      (store.entries[space] ?? []).filter((e) => e.path.startsWith(dir + '/')).map((e) => e.path.slice(dir.length + 1)),
    );
    const named = files.map((f) => {
      const name = uniqueName(f.name, taken);
      taken.add(name);
      return name === f.name ? f : new File([f], name, { type: f.type });
    });
    const names = named.map((f) => f.name);
    setUploadError(null);
    setUploading((cur) => [...cur, ...names]);
    try {
      const stored = await store.upload(space, dir, named);
      for (const e of stored) if (e.kind === 'file') add({ space, path: e.path });
      const failed = names.filter((n) => !stored.some((e) => e.path === `${dir}/${n}`));
      if (failed.length) setUploadError(`${failed.join(', ')}: ${useFilesStore.getState().error ?? 'upload failed'}`);
    } finally {
      setUploading((cur) => cur.filter((n) => !names.includes(n)));
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData?.files ?? []);
    if (!files.length || !space) return;
    e.preventDefault();
    void uploadAndAttach(files.map((f) => new File([f], pastedName(f.type), { type: f.type })));
  };

  const add = (a: AttachmentRef) =>
    setAttachments((cur) => (cur.some((x) => x.space === a.space && x.path === a.path) ? cur : [...cur, a]));

  const submit = () => {
    const text = value.trim();
    if (!text || streaming || disabled) return;
    const own = attachments;
    const fromPanel = picked.filter((p) => !own.some((a) => a.path === p.path));
    setValue('');
    setAttachments([]);
    useAttachStore.getState().clear();
    setShowPicked(false);
    void Promise.resolve(onSend(text, [...own, ...fromPanel])).then((ok) => {
      if (ok !== false) return;
      setValue((cur) => cur || text);
      setAttachments((cur) => (cur.length ? cur : own));
      if (fromPanel.length) useAttachStore.getState().add(fromPanel);
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const onDragOver = (e: DragEvent) => {
    const types = e.dataTransfer.types;
    if (types?.includes(DRAG_TYPE) || (space && types?.includes('Files'))) e.preventDefault();
  };

  const onDrop = (e: DragEvent) => {
    const dropped = Array.from(e.dataTransfer.files ?? []);
    if (dropped.length && space) {
      e.preventDefault();
      void uploadAndAttach(dropped);
      return;
    }
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
      {uploading.length > 0 && (
        <span role="status" className="px-1 text-xs text-slate-500">
          Uploading {uploading.join(', ')}
        </span>
      )}
      {uploadError && (
        <span role="alert" className="px-1 text-xs text-red-500">
          {uploadError}
        </span>
      )}
      {(attachments.length > 0 || picked.length > 0) && (
        <div className="flex flex-wrap gap-1">
          {attachments.map((a) => (
            <span
              key={`${a.space}:${a.path}`}
              className="flex max-w-56 items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-xs text-slate-600"
            >
              <span title={a.path} className="truncate">
                {a.path.split('/').pop() || a.path}
              </span>
              <button
                aria-label={`Remove ${a.path}`}
                onClick={() => setAttachments((cur) => cur.filter((x) => x !== a))}
                className="shrink-0 text-slate-400 hover:text-slate-700 max-md:p-3"
              >
                <X size={12} />
              </button>
            </span>
          ))}
          {picked.length > 0 && (
            <span className="flex items-center gap-1 rounded-full bg-indigo-50 px-2 py-0.5 text-xs text-slate-700">
              <button
                aria-expanded={showPicked}
                onClick={() => setShowPicked((v) => !v)}
                className="flex items-center gap-1 max-md:py-2"
              >
                <Folder size={12} className="text-indigo-500" />
                {picked.length === 1 ? '1 file from Files' : `${picked.length} files from Files`}
              </button>
              <button
                aria-label="Remove files from Files"
                onClick={() => {
                  useAttachStore.getState().clear();
                  setShowPicked(false);
                }}
                className="text-slate-400 hover:text-slate-700 max-md:p-3"
              >
                <X size={12} />
              </button>
            </span>
          )}
        </div>
      )}
      {showPicked && picked.length > 0 && (
        <ul aria-label="Files from Files" className="flex flex-col gap-0.5 px-1 text-xs text-slate-500">
          {picked.map((p) => (
            <li key={p.path} className="truncate" title={p.path}>
              {p.path}
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-end gap-1.5 rounded-2xl border border-slate-200 bg-white p-1.5 focus-within:border-indigo-300">
        {space && (
          <>
            <input
              ref={fileInput}
              type="file"
              multiple
              aria-label="Upload attachment"
              className="hidden"
              onChange={(e) => {
                const files = Array.from(e.target.files ?? []);
                e.target.value = '';
                void uploadAndAttach(files);
              }}
            />
            <button
              aria-label="Attach"
              onClick={() => fileInput.current?.click()}
              className="rounded-xl p-2.5 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
            >
              <Paperclip size={16} />
            </button>
          </>
        )}
        <textarea
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          rows={1}
          placeholder={placeholder}
          className="max-h-48 min-h-10 flex-1 resize-none bg-transparent px-1 py-2.5 text-sm text-slate-800 outline-none placeholder:text-slate-400"
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
