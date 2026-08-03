"use client";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

type Props = {
  content: string;
  role: "user" | "assistant";
};

export function ChatMessageContent({ content, role }: Props) {
  if (!content.trim()) return null;

  if (role === "user") {
    return <div className="whitespace-pre-wrap break-words text-white">{content}</div>;
  }

  return (
    <div className="chat-prose break-words text-[15px] leading-relaxed text-[var(--neo-ink)]">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          p: ({ children }) => <p className="mb-3 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="mb-3 list-disc space-y-1 pl-5 last:mb-0">{children}</ul>,
          ol: ({ children }) => <ol className="mb-3 list-decimal space-y-1 pl-5 last:mb-0">{children}</ol>,
          li: ({ children }) => <li className="leading-relaxed">{children}</li>,
          blockquote: ({ children }) => (
            <blockquote className="mb-3 border-l-2 border-[var(--neo-border)] pl-3 text-[var(--neo-muted)] last:mb-0">
              {children}
            </blockquote>
          ),
          code: ({ className, children }) => {
            const isBlock = Boolean(className);
            if (isBlock) {
              return (
                <pre className="mb-3 overflow-x-auto rounded-lg bg-stone-900/90 p-3 text-xs text-stone-100 last:mb-0">
                  <code>{children}</code>
                </pre>
              );
            }
            return (
              <code className="rounded bg-stone-200/80 px-1 py-0.5 text-[13px] font-medium text-stone-800">
                {children}
              </code>
            );
          },
          a: ({ href, children }) => (
            <a href={href} className="text-[var(--neo-primary)] underline underline-offset-2" target="_blank" rel="noreferrer">
              {children}
            </a>
          ),
          strong: ({ children }) => <strong className="font-semibold text-[var(--neo-ink)]">{children}</strong>,
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
