"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  LoaderCircle,
  Pencil,
  Plus,
  RotateCcw,
  Send,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ImageLightbox, type LightboxImage } from "@/components/image-lightbox";
import { ChatImageThumb } from "@/components/chat/chat-image-thumb";
import { chatApi, conversationsApi } from "@/lib/api";
import type { Conversation } from "@/lib/conversations";
import {
  CHAT_ACTIVE_SESSION_KEY,
  chatConversationTitle,
  createChatMessage,
  downloadTextFile,
  EMPTY_CHAT_STATE,
  exportChatAsMarkdown,
  exportChatAsText,
  estimateBase64Bytes,
  formatBytes,
  isChatConversationState,
  normalizeChatMessages,
  type ChatConversationState,
  type ChatMessage,
} from "@/lib/chat-conversations";
import { cn } from "@/lib/utils";

function toApiMessages(messages: ChatMessage[]) {
  return messages.map((m) => ({ role: m.role, content: m.content }));
}

function formatElapsed(ms?: number) {
  if (!ms || ms <= 0) return null;
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
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
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [lightboxIndex, setLightboxIndex] = useState(0);
  const [imgDimensions, setImgDimensions] = useState<Record<string, string>>({});
  const loadedImgKeys = useRef<Set<string>>(new Set());
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
    sessionStorage.setItem(CHAT_ACTIVE_SESSION_KEY, c.id);
    setMessages(normalizeChatMessages(c.state.messages));
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
    void (async () => {
      const list = await loadConversations();
      const stored = sessionStorage.getItem(CHAT_ACTIVE_SESSION_KEY);
      if (stored) {
        try {
          const c = await conversationsApi.get(stored);
          if (isChatConversationState(c.state)) {
            applyConversation(c);
            return;
          }
        } catch {
          sessionStorage.removeItem(CHAT_ACTIVE_SESSION_KEY);
        }
      }
      if (list.length > 0 && isChatConversationState(list[0].state)) {
        applyConversation(list[0] as Conversation);
      }
    })();
  }, [applyConversation, loadConversations]);

  useEffect(() => {
    for (const m of messages) {
      for (let j = 0; j < (m.images?.length ?? 0); j += 1) {
        const key = `${m.id}-img-${j}`;
        if (loadedImgKeys.current.has(key)) continue;
        loadedImgKeys.current.add(key);
        const b64 = m.images![j];
        const img = new Image();
        img.onload = () => {
          setImgDimensions((prev) => ({
            ...prev,
            [key]: `${img.naturalWidth}×${img.naturalHeight}`,
          }));
        };
        img.src = `data:image/png;base64,${b64}`;
      }
    }
  }, [messages]);

  const lightboxImages = useMemo((): LightboxImage[] => {
    const out: LightboxImage[] = [];
    for (const m of messages) {
      for (let j = 0; j < (m.images?.length ?? 0); j += 1) {
        const b64 = m.images![j];
        out.push({
          id: `${m.id}-img-${j}`,
          src: `data:image/png;base64,${b64}`,
          sizeLabel: formatBytes(estimateBase64Bytes(b64)),
          dimensions: imgDimensions[`${m.id}-img-${j}`],
        });
      }
    }
    return out;
  }, [messages, imgDimensions]);

  const openLightboxAt = (messageId: string, imageIndex: number) => {
    let flat = 0;
    for (const m of messages) {
      for (let j = 0; j < (m.images?.length ?? 0); j += 1) {
        if (m.id === messageId && j === imageIndex) {
          setLightboxIndex(flat);
          setLightboxOpen(true);
          return;
        }
        flat += 1;
      }
    }
  };

  const updateAndPersist = useCallback(
    async (id: string, nextMessages: ChatMessage[]) => {
      setMessages(nextMessages);
      await persistState(id, {
        kind: "chat",
        messages: nextMessages,
        model,
        stream,
      });
    },
    [model, persistState, stream],
  );

  const deleteConversation = async (id: string) => {
    if (!window.confirm("删除此对话？不可恢复。")) return;
    await conversationsApi.delete(id);
    if (conversationId === id) {
      setConversationId(null);
      setMessages([]);
      sessionStorage.removeItem(CHAT_ACTIVE_SESSION_KEY);
    }
    const list = await loadConversations();
    if (conversationId === id && list.length > 0) {
      applyConversation(list[0] as Conversation);
    }
  };

  const deleteMessage = async (index: number) => {
    if (!conversationId) return;
    const next = messages.filter((_, i) => i !== index);
    await updateAndPersist(conversationId, next);
  };

  const editMessage = async (index: number) => {
    const m = messages[index];
    if (!m || m.role !== "user") return;
    setInput(m.content);
    const next = messages.slice(0, index);
    if (conversationId) await updateAndPersist(conversationId, next);
  };

  const runCompletion = useCallback(
    async (activeId: string, apiMessages: ChatMessage[], withStream: boolean) => {
      const started = Date.now();
      let assistantText = "";

      const attachImage = (b64: string) => {
        setMessages((prev) => {
          const copy = [...prev];
          const last = copy[copy.length - 1];
          if (last?.role === "assistant") {
            copy[copy.length - 1] = { ...last, images: [...(last.images ?? []), b64] };
          }
          return copy;
        });
        scrollToBottom();
      };

      if (withStream) {
        setMessages((prev) => [...prev, createChatMessage("assistant", "")]);
      }

      await chatApi.streamCompletion(
        {
          model,
          stream: withStream,
          messages: toApiMessages(apiMessages),
        },
        (delta) => {
          assistantText += delta;
          if (withStream) {
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

      const elapsedMs = Date.now() - started;
      let finalMessages: ChatMessage[] = [];

      if (!withStream) {
        finalMessages = [
          ...apiMessages,
          createChatMessage("assistant", assistantText || "（空响应）"),
        ];
        finalMessages[finalMessages.length - 1].elapsedMs = elapsedMs;
        setMessages(finalMessages);
      } else {
        setMessages((prev) => {
          const copy = [...prev];
          const last = copy[copy.length - 1];
          if (last?.role === "assistant") {
            copy[copy.length - 1] = { ...last, elapsedMs };
          }
          finalMessages = copy;
          return copy;
        });
      }

      await persistState(activeId, {
        kind: "chat",
        messages: finalMessages,
        model,
        stream,
      });
    },
    [model, persistState, stream],
  );

  const onSend = useCallback(async () => {
    const text = input.trim();
    if (!text || streaming) return;

    let activeId = conversationId;
    const userMsg = createChatMessage("user", text);
    const nextMessages: ChatMessage[] = [...messages, userMsg];

    setError("");
    setInput("");
    setMessages(nextMessages);
    setStreaming(true);
    scrollToBottom();

    try {
      if (!activeId) {
        const created = await conversationsApi.create({
          title: chatConversationTitle(nextMessages),
          state: { kind: "chat", messages: nextMessages, model, stream },
        });
        activeId = created.id;
        setConversationId(activeId);
        sessionStorage.setItem(CHAT_ACTIVE_SESSION_KEY, activeId);
        await loadConversations();
      } else {
        await persistState(activeId, {
          kind: "chat",
          messages: nextMessages,
          model,
          stream,
        });
      }

      await runCompletion(activeId, nextMessages, stream);
    } catch (err) {
      setError(err instanceof Error ? err.message : "对话失败");
      if (stream) {
        setMessages((prev) =>
          prev[prev.length - 1]?.role === "assistant" && !prev[prev.length - 1]?.content
            ? prev.slice(0, -1)
            : prev,
        );
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
    runCompletion,
  ]);

  const resendFrom = async (index: number) => {
    const m = messages[index];
    if (!m || m.role !== "user" || !conversationId || streaming) return;
    const truncated = messages.slice(0, index + 1);
    setMessages(truncated);
    setStreaming(true);
    setError("");
    try {
      await persistState(conversationId, {
        kind: "chat",
        messages: truncated,
        model,
        stream,
      });
      await runCompletion(conversationId, truncated, stream);
    } catch (err) {
      setError(err instanceof Error ? err.message : "重发失败");
    } finally {
      setStreaming(false);
    }
  };

  const onExport = (format: "txt" | "md") => {
    const title = chatConversationTitle(messages);
    const safe = title.replace(/[^\w\u4e00-\u9fff-]+/g, "_").slice(0, 32);
    if (format === "txt") {
      downloadTextFile(`${safe || "chat"}.txt`, exportChatAsText(messages));
    } else {
      downloadTextFile(`${safe || "chat"}.md`, exportChatAsMarkdown(messages), "text/markdown;charset=utf-8");
    }
  };

  return (
    <div className="flex h-full min-h-0">
      {sidebarOpen ? (
        <aside className="flex w-56 shrink-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface-muted)]">
          <div className="flex items-center justify-between border-b border-[var(--neo-border)] px-3 py-2">
            <span className="text-xs font-medium text-[var(--neo-muted)]">对话记录</span>
            <div className="flex items-center gap-0.5">
              <Button
                variant="ghost"
                size="sm"
                className="h-7 w-7 p-0"
                title="导出 txt"
                disabled={messages.length === 0}
                onClick={() => onExport("txt")}
              >
                <Download className="h-3.5 w-3.5" />
              </Button>
              <Button variant="outline" size="sm" className="h-7 px-2" onClick={() => void createConversation()}>
                <Plus className="h-3.5 w-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 w-7 p-0"
                title="收起侧栏"
                onClick={() => setSidebarOpen(false)}
              >
                <ChevronLeft className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
          <div className="flex-1 space-y-1 overflow-y-auto p-2">
            {conversations.map((c) => (
              <div
                key={c.id}
                className={cn(
                  "group flex items-start gap-1 rounded-lg border px-2 py-2 transition-colors",
                  conversationId === c.id
                    ? "border-[var(--neo-primary)] bg-white"
                    : "border-transparent hover:bg-white/80",
                )}
              >
                <button
                  type="button"
                  onClick={() => applyConversation(c)}
                  className="min-w-0 flex-1 text-left text-sm"
                >
                  <p className="line-clamp-2 font-medium text-[var(--neo-ink)]">{c.title}</p>
                </button>
                <button
                  type="button"
                  className="rounded p-1 text-[var(--neo-muted)] opacity-0 transition hover:bg-rose-50 hover:text-rose-600 group-hover:opacity-100"
                  title="删除对话"
                  onClick={() => void deleteConversation(c.id)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            ))}
          </div>
          {messages.length > 0 ? (
            <div className="border-t border-[var(--neo-border)] p-2">
              <Button
                variant="ghost"
                size="sm"
                className="h-7 w-full text-xs"
                onClick={() => onExport("md")}
              >
                导出 Markdown
              </Button>
            </div>
          ) : null}
        </aside>
      ) : (
        <div className="flex w-10 shrink-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface-muted)]">
          <Button
            variant="ghost"
            size="sm"
            className="h-10 w-full"
            title="展开侧栏"
            onClick={() => setSidebarOpen(true)}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      )}

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
              key={m.id}
              className={cn(
                "group max-w-[85%]",
                m.role === "user" ? "ml-auto" : "mr-auto",
              )}
            >
              <div
                className={
                  m.role === "user"
                    ? "whitespace-pre-wrap rounded-2xl rounded-br-md bg-[var(--neo-primary-gradient)] px-4 py-2.5 text-sm text-white"
                    : "whitespace-pre-wrap rounded-2xl rounded-bl-md border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-4 py-2.5 text-sm text-[var(--neo-ink)]"
                }
              >
                <div className="mb-1 flex items-center justify-between gap-2 text-[10px] font-medium opacity-70">
                  <span>{m.role === "user" ? "你" : "助手"}</span>
                  {m.elapsedMs ? (
                    <span className="tabular-nums">耗时 {formatElapsed(m.elapsedMs)}</span>
                  ) : streaming && i === messages.length - 1 && m.role === "assistant" ? (
                    <span>生成中…</span>
                  ) : null}
                </div>
                {m.content || (streaming && i === messages.length - 1 && m.role === "assistant" ? "…" : "")}
                {m.images?.map((b64, j) => (
                  <ChatImageThumb key={j} b64={b64} onOpen={() => openLightboxAt(m.id, j)} />
                ))}
              </div>
              <div
                className={cn(
                  "mt-1 flex flex-wrap gap-1 opacity-0 transition-opacity group-hover:opacity-100",
                  m.role === "user" ? "justify-end" : "justify-start",
                )}
              >
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-6 px-2 text-[10px]"
                  disabled={streaming}
                  onClick={() => void deleteMessage(i)}
                >
                  <Trash2 className="mr-1 size-3" />
                  删除
                </Button>
                {m.role === "user" ? (
                  <>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 px-2 text-[10px]"
                      disabled={streaming}
                      onClick={() => void editMessage(i)}
                    >
                      <Pencil className="mr-1 size-3" />
                      编辑
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 px-2 text-[10px]"
                      disabled={streaming}
                      onClick={() => void resendFrom(i)}
                    >
                      <RotateCcw className="mr-1 size-3" />
                      重发
                    </Button>
                  </>
                ) : null}
              </div>
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

      <ImageLightbox
        images={lightboxImages}
        currentIndex={lightboxIndex}
        open={lightboxOpen}
        onOpenChange={setLightboxOpen}
        onIndexChange={setLightboxIndex}
      />
    </div>
  );
}
