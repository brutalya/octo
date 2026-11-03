import { useState, type KeyboardEvent } from 'react';
import { ArrowUp, Square } from 'lucide-react';

interface Props {
  streaming: boolean;
  onSend: (text: string) => void;
  onStop: () => void;
}

export function ChatInput({ streaming, onSend, onStop }: Props) {
  const [value, setValue] = useState('');

  const submit = () => {
    const text = value.trim();
    if (!text || streaming) return;
    onSend(text);
    setValue('');
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="mx-auto flex w-full max-w-3xl items-end gap-2 px-4 pb-4">
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
          onClick={submit}
          className="rounded-xl bg-indigo-500 p-2.5 text-slate-50 hover:bg-indigo-600"
        >
          <ArrowUp size={16} />
        </button>
      )}
    </div>
  );
}
