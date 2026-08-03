"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { LoaderCircle, Plus, Send } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { chatApi, conversationsApi } from "@/lib/api";
import type { Conversation } from "@/lib/conversations";
import {
  chatConversationTitle,
  EMPTY_CHAT_STATE,
  isChatConversationState,
  type ChatConversationState,
  type ChatMessage,
} from "@/lib/chat-conversations";
import { cn } from "@/lib/utils";

function toApiMessages(messages: ChatMessage[]) {
  return messages.map((m) => ({ role: m.role, content: m.content }));
}

export function ChatWorkbench() {
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState(EMPTY_CHAT_STATE.model);
  const [stream, setStream] = useState(EMPTY_CHAT_STATE.stream);
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = () => {
    requestAnimationFrame(() => {
      listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: "smooth" });
    });
  };

  const loadConversations = useCallback(async () => {
    const list = await conversationsApi.list();
    const chatOnly = list.filter((c) => isChatConversationState(c.state));
    setConversations(chatOnly as Conversation[]);
    return chatOnly;
  }, []);

  const persistState = useCallback(
    async (id: string, state: ChatConversationState) => {
      await conversationsApi.patch(id, {
        title: chatConversationTitle(state.messages),
        state,
      });
      await loadConversations();
    },
    [loadConversations],
  );

  const applyConversation = useCallback((c: Conversation) => {
    if (!isChatConversationState(c.state)) return;
    setConversationId(c.id);
    setMessages(c.state.messages);
    setModel(c.state.model ?? EMPTY_CHAT_STATE.model);
    setStream(c.state.stream ?? true);
    setError("");
  }, []);

  const createConversation = useCallback(async () => {
    const created = await conversationsApi.create({
      title: "新对话",
      state: EMPTY_CHAT_STATE,
    });
    await loadConversations();
    applyConversation(created);
  }, [applyConversation, loadConversations]);

  useEffect(() => {
    void loadConversations().then((list) => {
      if (list.length > 0 && isChatConversationState(list[0].state)) {
        applyConversation(list[0] as Conversation);
      }
    });
  }, [applyConversation, loadConversations]);

  const onSend = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;

    let activeId = conversationId;
    if (!activeId) {
      const created = await conversationsApi.create({
        title: chatConversationTitle([{ role: "user", content: text }]),
        state: { ...EMPTY_CHAT_STATE, model, stream, messages: [] },
      });
      activeId = created.id;
      setConversationId(activeId);
      await loadConversations();
    }

    setError("");
    const nextMessages: ChatMessage[] = [...messages, { role: "user", content: text }];
    setMessages(nextMessages);
    setInput("");
    setStreaming(true);
    if (stream) {
      setMessages((prev) => [...prev, { role: "assistant", content: "", images: [] }]);
    }
    scrollToBottom();

    const attachImage = (b64: string) => {
      setMessages((prev) => {
        const copy = [...prev];
        const last = copy[copy.length - 1];
        if (last?.role === "assistant") {
          const images = [...(last.images ?? []), b64];
          copy[copy.length - 1] = { ...last, images };
        }
        return copy;
      });
      scrollToBottom();
    };

    try {
      let assistantText = "";
      await chatApi.streamCompletion(
        {
          model,
          stream,
          messages: toApiMessages(nextMessages),
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
        attachImage,
      );
      if (!stream) {
        setMessages((prev) => [
          ...prev,
          { role: "assistant", content: assistantText || "（空响应）", images: [] },
        ]);
      }
      if (activeId) {
        setMessages((current) => {
          void persistState(activeId, {
            kind: "chat",
            messages: current,
            model,
            stream,
          });
          return current;
        });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "对话失败");
      if (stream) {
        setMessages((prev) => (prev[prev.length - 1]?.role === "assistant" ? prev.slice(0, -1) : prev));
      }
    } finally {
      setStreaming(false);
    }
  }, [
    input,
    streaming,
    messages,
    model,
    stream,
    conversationId,
    loadConversations,
    persistState,
  ]);

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-56 shrink-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface-muted)]">
        <div className="flex items-center justify-between border-b border-[var(--neo-border)] px-3 py-2">
          <span className="text-xs font-medium text-[var(--neo-muted)]">对话记录</span>
          <Button variant="outline" size="sm" className="h-7 px-2" onClick={() => void createConversation()}>
            <Plus className="h-3.5 w-3.5" />
          </Button>
        </div>
        <div className="flex-1 space-y-1 overflow-y-auto p-2">
          {conversations.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => applyConversation(c)}
              className={cn(
                "w-full rounded-lg border px-2.5 py-2 text-left text-sm transition-colors",
                conversationId === c.id
                  ? "border-[var(--neo-primary)] bg-white"
                  : "border-transparent hover:bg-white/80",
              )}
            >
              <p className="line-clamp-2 font-medium text-[var(--neo-ink)]">{c.title}</p>
            </button>
          ))}
        </div>
      </aside>

      <div className="flex min-h-0 flex-1 flex-col">
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
              发送消息开始多轮对话。支持 <code className="text-xs">@Create image</code> 或{" "}
              <code className="text-xs">/image 提示词</code> 对话内生图。
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
              {m.images?.map((b64, j) => (
                <img
                  key={j}
                  src={`data:image/png;base64,${b64}`}
                  alt="生成图"
                  className="mt-2 max-h-80 rounded-lg border border-[var(--neo-border)] object-contain"
                />
              ))}
            </div>
          ))}
        </div>
        {error ? <p className="px-4 text-sm text-red-600">{error}</p> : null}
        <div className="flex gap-2 border-t border-[var(--neo-border)] p-3">
          <Input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="输入消息…（/image 日落海边）"
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
    </div>
  );
}
