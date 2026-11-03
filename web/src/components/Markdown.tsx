import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

export function Markdown({ children }: { children: string }) {
  return (
    <div className="md text-sm leading-relaxed text-slate-800 [&_a]:text-indigo-500 [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-slate-300 [&_blockquote]:pl-3 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_table]:border-collapse [&_td]:border [&_td]:border-slate-200 [&_td]:px-2 [&_th]:border [&_th]:border-slate-200 [&_th]:px-2 [&_ul]:list-disc [&_ul]:pl-5">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children: c }) => (
            <pre className="my-2 overflow-x-auto rounded-lg bg-slate-100 p-3 font-mono text-xs">{c}</pre>
          ),
          code: ({ className, children: c }) => (
            <code className={`font-mono ${className ?? 'rounded bg-slate-100 px-1 text-[0.85em]'}`}>{c}</code>
          ),
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
