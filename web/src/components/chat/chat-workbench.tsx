"use client";

import { useCallback, useRef, useState } from "react";
import { LoaderCircle, Send } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { chatApi } from "@/lib/api";

type Message = { role: "user" | "assistant"; content: string };

export function ChatWorkbench() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState("gpt-4o-mini");
  const [stream, setStream] = useState(true);
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = () => {
    requestAnimationFrame(() => {
      listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: "smooth" });
    });
  };

  const onSend = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;
    setError("");
    const nextMessages: Message[] = [...messages, { role: "user", content: text }];
    setMessages(nextMessages);
    setInput("");
    setStreaming(true);
    if (stream) {
      setMessages((prev) => [...prev, { role: "assistant", content: "" }]);
    }
    scrollToBottom();

    try {
      let assistantText = "";
      await chatApi.streamCompletion(
        {
          model,
          stream,
          messages: nextMessages.map((m) => ({ role: m.role, content: m.content })),
        },
        (delta) => {
          assistantText += delta;
          if (stream) {
            setMessages((prev) => {
              const copy = [...prev];
              const last = copy[copy.length - 1];
              if (last?.role === "assistant") {
                copy[copy.length - 1] = { ...last, content: last.content + delta };
              }
              return copy;
            });
          }
          scrollToBottom();
        },
      );
      if (!stream) {
        setMessages((prev) => [...prev, { role: "assistant", content: assistantText || "（空响应）" }]);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "对话失败");
      if (stream) {
        setMessages((prev) => (prev[prev.length - 1]?.role === "assistant" ? prev.slice(0, -1) : prev));
      }
    } finally {
      setStreaming(false);
    }
  }, [input, streaming, messages, model, stream]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-3 border-b border-[var(--neo-border)] px-4 py-2">
        <label className="text-xs text-[var(--neo-muted)]">模型</label>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className="neo-input h-8 rounded-lg px-2 text-sm"
        >
          <option value="gpt-4o">gpt-4o</option>
          <option value="gpt-4o-mini">gpt-4o-mini</option>
          <option value="o4-mini">o4-mini</option>
        </select>
        <label className="ml-auto flex items-center gap-1.5 text-xs text-[var(--neo-muted)]">
          <input type="checkbox" checked={stream} onChange={(e) => setStream(e.target.checked)} />
          流式 SSE
        </label>
      </div>
      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto p-4">
        {messages.length === 0 ? (
          <p className="py-8 text-center text-sm text-[var(--neo-muted)]">
            发送消息开始对话。经 TNexus API 代理至 Gateway 数据面，并写入对话用量统计。
          </p>
        ) : null}
        {messages.map((m, i) => (
          <div
            key={i}
            className={
              m.role === "user"
                ? "ml-auto max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-[var(--neo-primary-gradient)] px-4 py-2.5 text-sm text-white"
                : "mr-auto max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-bl-md border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-4 py-2.5 text-sm text-[var(--neo-ink)]"
            }
          >
            <div className="mb-1 text-[10px] font-medium opacity-70">{m.role === "user" ? "你" : "助手"}</div>
            {m.content || (streaming && i === messages.length - 1 ? "…" : "")}
          </div>
        ))}
      </div>
      {error ? <p className="px-4 text-sm text-red-600">{error}</p> : null}
      <div className="flex gap-2 border-t border-[var(--neo-border)] p-3">
        <Input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="输入消息…"
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void onSend();
            }
          }}
          disabled={streaming}
        />
        <Button className="shrink-0 gap-1.5" disabled={streaming || !input.trim()} onClick={() => void onSend()}>
          {streaming ? <LoaderCircle className="size-4 animate-spin" /> : <Send className="size-4" />}
          发送
        </Button>
      </div>
    </div>
  );
}
