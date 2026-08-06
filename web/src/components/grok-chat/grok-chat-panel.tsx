"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Bot, Eraser, Image as ImageIcon, Loader2, Send, Sparkles } from "lucide-react";
import { chatApi, sniffImageMime } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { ChatImageThumb } from "@/components/chat/chat-image-thumb";
import { ImageLightbox, type LightboxImage } from "@/components/image-lightbox";
import { estimateBase64Bytes, formatBytes } from "@/lib/chat-conversations";

/** Grok 对话模型选项（gateway 非 OCR 时会归一化到上游 grok-chat；下拉保留可读性/未来透传）。 */
export const GROK_CHAT_MODELS = [
  "grok-chat-fast",
  "grok-3",
  "grok-4.5",
  "grok-4.5-build-free",
] as const;

export type GrokChatModel = (typeof GROK_CHAT_MODELS)[number];

interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  error?: boolean;
  /** 生图结果（b64，无 data URI 前缀）。 */
  images?: string[];
}

let nextId = 1;

function makeMessage(role: ChatMessage["role"], content: string): ChatMessage {
  return { id: nextId++, role, content };
}

/** 流式对话面板：内存会话（无持久化），走既有 chatApi（/v1/chat/completions SSE）。 */
export function GrokChatPanel() {
  const [model, setModel] = useState<GrokChatModel>("grok-chat-fast");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const [imagining, setImagining] = useState(false);
  const [imageCount, setImageCount] = useState(1);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [lightboxIndex, setLightboxIndex] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(sending);
  useEffect(() => {
    sendingRef.current = sending;
  }, [sending]);
  const imaginingRef = useRef(imagining);
  useEffect(() => {
    imaginingRef.current = imagining;
  }, [imagining]);

  const appendAssistant = useCallback((content: string, isError = false) => {
    setMessages((prev) => {
      const last = prev[prev.length - 1];
      if (last?.role === "assistant" && !last.error) {
        const next = [...prev];
        next[next.length - 1] = { ...last, content: last.content + content };
        return next;
      }
      return [...prev, { id: nextId++, role: "assistant" as const, content, error: isError }];
    });
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, sending]);

  const send = useCallback(async () => {
    const text = draft.trim();
    if (!text || sendingRef.current) return;
    setDraft("");
    setError("");
    setMessages((prev) => [...prev, makeMessage("user", text)]);
    setSending(true);
    // 内存会话：流式开始时先放一个空的 assistant 气泡，delta 逐字追加。
    setMessages((prev) => [...prev, makeMessage("assistant", "")]);
    try {
      const history = [...messages, makeMessage("user", text)].map((m) => ({
        role: m.role,
        content: m.content,
      }));
      await chatApi.streamCompletion(
        { model, messages: history, stream: true },
        (delta) => appendAssistant(delta),
      );
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      appendAssistant(msg, true);
    } finally {
      setSending(false);
    }
  }, [draft, messages, model, appendAssistant]);

  const clear = useCallback(() => {
    if (sendingRef.current || imaginingRef.current) return;
    setMessages([]);
    setError("");
  }, []);

  const lightboxImages = useMemo((): LightboxImage[] => {
    const out: LightboxImage[] = [];
    for (const m of messages) {
      for (let j = 0; j < (m.images?.length ?? 0); j += 1) {
        const b64 = m.images![j];
        out.push({
          id: `${m.id}-img-${j}`,
          src: `data:image/${sniffImageMime(b64)};base64,${b64}`,
          sizeLabel: formatBytes(estimateBase64Bytes(b64)),
        });
      }
    }
    return out;
  }, [messages]);

  const openLightbox = useCallback((imageIndex: number) => {
    setLightboxIndex(imageIndex);
    setLightboxOpen(true);
  }, []);

  /** 生图：prompt 取 draft，结果以独立 assistant 消息呈现（不走流式）。 */
  const imagine = useCallback(async () => {
    const text = draft.trim();
    if (!text || sendingRef.current || imaginingRef.current) return;
    setDraft("");
    setError("");
    setMessages((prev) => [...prev, makeMessage("user", text)]);
    setImagining(true);
    try {
      const items = await chatApi.generateImage(text, imageCount);
      if (items.length === 0) throw new Error("生图返回空结果");
      setMessages((prev) => [
        ...prev,
        {
          id: nextId++,
          role: "assistant" as const,
          content: `已生成 ${items.length} 张图片`,
          images: items,
        },
      ]);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setMessages((prev) => [
        ...prev,
        { id: nextId++, role: "assistant" as const, content: msg, error: true },
      ]);
    } finally {
      setImagining(false);
    }
  }, [draft, imageCount]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-[var(--neo-surface)]">
      {/* 头部：模型选择 + 清空 */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--neo-border)] px-4 py-2">
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-[var(--neo-primary)] text-white shadow-bl-sm">
            <Sparkles className="size-4" />
          </span>
          <select
            value={model}
            onChange={(e) => setModel(e.target.value as GrokChatModel)}
            className="neo-input h-8 rounded-lg px-2 text-sm font-medium text-[var(--neo-ink)] focus-visible:outline-none"
            aria-label="Grok 模型"
          >
            {GROK_CHAT_MODELS.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
        <Button variant="ghost" size="sm" onClick={clear} disabled={sending || imagining || messages.length === 0}>
          <Eraser className="size-4" />
          清空
        </Button>
      </div>

      {/* 消息区 */}
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-6">
        {messages.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <div className="flex size-14 items-center justify-center rounded-2xl bg-gradient-to-br from-[var(--color-brand-lavender)] to-[var(--color-brand-pink)] text-white shadow-bl">
              <Bot className="size-7" />
            </div>
            <p className="text-base font-medium text-[var(--neo-ink)]">开始一段 Grok 对话</p>
            <p className="max-w-md text-sm leading-relaxed text-[var(--neo-muted)]">
              对话走 gateway <code className="rounded bg-[var(--neo-surface-muted)] px-1">/v1/chat/completions</code>
              （SSE 流式）；生图走{" "}
              <code className="rounded bg-[var(--neo-surface-muted)] px-1">/v1/images/generations</code>
              （需 gateway 开启 GROK_IMAGE_ENABLED=1 且 browser-bridge 就绪）。当前为内存会话，刷新后清空。
            </p>
          </div>
        ) : (
          <div className="mx-auto flex max-w-3xl flex-col gap-4">
            {messages.map((m) => (
              <div
                key={m.id}
                className={cn("flex items-start gap-2", m.role === "user" && "flex-row-reverse")}
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full text-xs font-semibold text-white",
                    m.role === "user" ? "bg-[var(--neo-primary-deep)]" : "bg-[var(--neo-primary)]",
                  )}
                >
                  {m.role === "user" ? "我" : "G"}
                </span>
                <div
                  className={cn(
                    "max-w-[85%] whitespace-pre-wrap break-words rounded-2xl px-4 py-2.5 text-sm leading-relaxed shadow-sm",
                    m.role === "user"
                      ? "rounded-tr-sm bg-[var(--neo-primary)] text-white"
                      : cn(
                          "rounded-tl-sm border border-[var(--neo-border)] bg-white text-[var(--neo-ink)]",
                          m.error && "border-rose-200 bg-rose-50 text-rose-700",
                        ),
                  )}
                >
                  {m.content || <span className="text-[var(--neo-muted)]">…</span>}
                  {m.images && m.images.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-2">
                      {m.images.map((b64, j) => {
                        const imageIndex = lightboxImages.findIndex((li) => li.id === `${m.id}-img-${j}`);
                        return (
                          <ChatImageThumb
                            key={j}
                            b64={b64}
                            mime={sniffImageMime(b64)}
                            onOpen={() => openLightbox(imageIndex >= 0 ? imageIndex : 0)}
                          />
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            ))}
            {sending && (
              <div className="flex items-center gap-2 pl-10 text-sm text-[var(--neo-muted)]">
                <Loader2 className="size-4 animate-spin" />
                生成中…
              </div>
            )}
            {imagining && (
              <div className="flex items-center gap-2 pl-10 text-sm text-[var(--neo-muted)]">
                <Loader2 className="size-4 animate-spin" />
                正在生成图片…
              </div>
            )}
          </div>
        )}
      </div>

      {/* 输入区 */}
      <div className="shrink-0 border-t border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-4 py-3">
        <div className="mx-auto flex max-w-3xl items-end gap-2 rounded-2xl border border-[var(--neo-border)] bg-white p-2 shadow-sm">
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="输入消息，Enter 发送，Shift+Enter 换行"
            className="min-h-[44px] max-h-40 flex-1 resize-none border-none bg-transparent px-2 py-2 text-[15px] leading-relaxed shadow-none placeholder:text-[var(--neo-muted)] focus-visible:outline-none"
            disabled={sending}
          />
          <select
            value={imageCount}
            onChange={(e) => setImageCount(Number(e.target.value))}
            className="neo-input h-9 rounded-lg px-1.5 text-sm text-[var(--neo-ink)] focus-visible:outline-none"
            aria-label="生图数量"
            title="生图数量"
            disabled={sending || imagining}
          >
            {[1, 2, 3, 4].map((n) => (
              <option key={n} value={n}>
                {n}张
              </option>
            ))}
          </select>
          <Button
            size="icon"
            onClick={() => void imagine()}
            disabled={sending || imagining || !draft.trim()}
            aria-label="生成图片"
            title="生成图片（/v1/images/generations）"
          >
            {imagining ? <Loader2 className="size-4 animate-spin" /> : <ImageIcon className="size-4" />}
          </Button>
          <Button
            size="icon"
            onClick={() => void send()}
            disabled={sending || imagining || !draft.trim()}
            aria-label="发送"
          >
            {sending ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
          </Button>
        </div>
        <ImageLightbox
          images={lightboxImages}
          currentIndex={lightboxIndex}
          open={lightboxOpen}
          onOpenChange={setLightboxOpen}
          onIndexChange={setLightboxIndex}
        />
        {error && !sending && !imagining && (
          <p className="mx-auto mt-2 max-w-3xl text-xs text-rose-600">{error}</p>
        )}
        <p className="mx-auto mt-2 max-w-3xl text-[11px] text-[var(--neo-muted)]">
          模型下拉为非 OCR 文本通道；gateway 会归一化到上游 grok-chat。生图需 gateway 开启
          GROK_IMAGE_ENABLED=1 且 browser-bridge 就绪，否则返回 500/503。
        </p>
      </div>
    </div>
  );
}
