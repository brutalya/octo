import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

export function Markdown({ children }: { children: string }) {
  return (
    <div className="md min-w-0 max-w-full text-sm leading-relaxed text-slate-800 [overflow-wrap:anywhere] [&_table]:block [&_table]:max-w-full [&_table]:overflow-x-auto [&_:not(pre)>code]:rounded [&_:not(pre)>code]:bg-slate-100 [&_:not(pre)>code]:px-1 [&_:not(pre)>code]:text-[0.85em] [&_a]:text-indigo-500 [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-slate-300 [&_blockquote]:pl-3 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_table]:border-collapse [&_td]:border [&_td]:border-slate-200 [&_td]:px-2 [&_th]:border [&_th]:border-slate-200 [&_th]:px-2 [&_ul]:list-disc [&_ul]:pl-5">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children: c }) => (
            <pre className="my-2 overflow-x-auto rounded-lg bg-slate-100 p-3 font-mono text-xs">{c}</pre>
          ),
          code: ({ className, children: c }) => <code className={`font-mono ${className ?? ''}`}>{c}</code>,
          a: ({ href, children: c }) => (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {c}
            </a>
          ),
          // Images are never fetched (tracking pixels); show the alt text as a link instead.
          img: ({ src, alt }) =>
            src ? (
              <a href={typeof src === 'string' ? src : undefined} target="_blank" rel="noopener noreferrer">
                {alt || src}
              </a>
            ) : (
              <span>{alt}</span>
            ),
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
