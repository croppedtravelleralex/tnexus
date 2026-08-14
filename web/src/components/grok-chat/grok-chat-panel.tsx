"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Bot, Eraser, Image as ImageIcon, Loader2, Paperclip, Send, Sparkles } from "lucide-react";
import { grokApi, sniffImageMime, type GrokChatMessage, type GrokChatContentPart } from "@/lib/grok-api";
import { useAuth } from "@/lib/auth";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { ChatImageThumb } from "@/components/chat/chat-image-thumb";
import { ChatMessageContent } from "@/components/chat/chat-message-content";
import { ImageLightbox, type LightboxImage } from "@/components/image-lightbox";
import { estimateBase64Bytes, formatBytes } from "@/lib/chat-conversations";
import { formatReplyDuration, looksLikeImaginePrompt } from "@/lib/grok-text";

/** Grok 对话模型选项（gateway 非 OCR 时会归一化到上游 grok-chat；下拉保留可读性/未来透传）。 */
export const GROK_CHAT_MODELS = [
  "grok-chat-fast",
  "grok-vision-ocr",
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
  images?: string[];
  /** data URL 附件（纯 HTTP OCR 默认走 upload-file） */
  attachments?: string[];
  /** 助手回复耗时（毫秒） */
  durationMs?: number;
}

let nextId = 1;

function makeMessage(role: ChatMessage["role"], content: string): ChatMessage {
  return { id: nextId++, role, content };
}

/** 流式对话面板：可走外部会话持久化。走 grokApi，默认经 TNexus `/api/grok/v1` 代理
 *  转发到 grok2api-rs 的 `/v1/chat/completions`（SSE）；:8000 不对浏览器暴露。 */
export type GrokChatPanelProps = {
  /** 切换会话时变化，用于重置本地消息 */
  sessionKey?: string | null;
  initialModel?: GrokChatModel;
  initialMessages?: Array<{
    id?: string | number;
    role: "user" | "assistant";
    content: string;
    error?: boolean;
    images?: string[];
    attachments?: string[];
    durationMs?: number;
  }>;
  /** 消息或模型变更后回调（供 workbench 写 conversations API） */
  onPersist?: (state: {
    model: GrokChatModel;
    messages: ChatMessage[];
    lastAccountId?: number | null;
  }) => void;
  /** 从某条用户消息起重新发送 */
  resendIndex?: number | null;
  onResendDone?: () => void;
  initialLastAccountId?: number | null;
  onResendRequest?: (messageIndex: number) => void;
};

export function GrokChatPanel({
  sessionKey = null,
  initialModel = "grok-chat-fast",
  initialMessages = [],
  onPersist,
  resendIndex = null,
  onResendDone,
  initialLastAccountId = null,
  onResendRequest,
}: GrokChatPanelProps = {}) {
  const { user } = useAuth();
  const [model, setModel] = useState<GrokChatModel>(initialModel);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [activeAccountId, setActiveAccountId] = useState<number | null>(initialLastAccountId);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const [imagining, setImagining] = useState(false);
  const [imageCount, setImageCount] = useState(1);
  const [aspectRatio, setAspectRatio] = useState("1:1");
  const [pendingImages, setPendingImages] = useState<string[]>([]);
  const fileRef = useRef<HTMLInputElement>(null);
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
  const [liveMs, setLiveMs] = useState(0);
  const busyStartedAt = useRef<number | null>(null);
  useEffect(() => {
    if (!sending && !imagining) {
      busyStartedAt.current = null;
      setLiveMs(0);
      return;
    }
    if (busyStartedAt.current == null) busyStartedAt.current = Date.now();
    const id = window.setInterval(() => {
      if (busyStartedAt.current != null) setLiveMs(Date.now() - busyStartedAt.current);
    }, 200);
    return () => window.clearInterval(id);
  }, [sending, imagining]);

  const hydrateFromInitial = useCallback(() => {
    setModel(initialModel);
    setMessages(
      initialMessages.map((m) => ({
        id: typeof m.id === "number" ? m.id : nextId++,
        role: m.role,
        content: m.content,
        error: m.error,
        images: m.images,
        attachments: m.attachments,
        durationMs: typeof m.durationMs === "number" ? m.durationMs : undefined,
      })),
    );
    setError("");
    setDraft("");
    setPendingImages([]);
  }, [initialModel, initialMessages]);

  useEffect(() => {
    hydrateFromInitial();
    setActiveAccountId(initialLastAccountId);
  }, [sessionKey, hydrateFromInitial, initialLastAccountId]);

  const persistNow = useCallback(
    (nextMessages: ChatMessage[], nextModel: GrokChatModel = model, accountId = activeAccountId) => {
      onPersist?.({ model: nextModel, messages: nextMessages, lastAccountId: accountId });
    },
    [model, onPersist, activeAccountId],
  );

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

  const stampDuration = useCallback((ms: number) => {
    setMessages((prev) => {
      const next = [...prev];
      for (let i = next.length - 1; i >= 0; i -= 1) {
        if (next[i].role === "assistant") {
          next[i] = { ...next[i], durationMs: ms };
          break;
        }
      }
      return next;
    });
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, sending]);

  const buildUpstreamMessages = useCallback((history: ChatMessage[]): GrokChatMessage[] => {
    return history.map((m) => {
      if (m.role === "user" && (m.attachments?.length || 0) > 0) {
        const parts: GrokChatContentPart[] = [
          { type: "text", text: m.content },
          ...m.attachments!.map((url) => ({ type: "image_url" as const, image_url: { url } })),
        ];
        return { role: m.role, content: parts };
      }
      return { role: m.role, content: m.content };
    });
  }, []);

  const runCompletion = useCallback(
    async (history: ChatMessage[]) => {
      const upstreamModel =
        model === "grok-vision-ocr" || history.some((m) => (m.attachments?.length ?? 0) > 0)
          ? "grok-vision-ocr"
          : model;
      const { accountId } = await grokApi.streamCompletion(
        { model: upstreamModel, messages: buildUpstreamMessages(history), stream: true },
        (delta) => appendAssistant(delta),
      );
      if (accountId != null) setActiveAccountId(accountId);
      return accountId;
    },
    [model, appendAssistant, buildUpstreamMessages],
  );

  const runImagine = useCallback(
    async (text: string) => {
      if (!text || sendingRef.current || imaginingRef.current) return;
      setError("");
      setMessages((prev) => [...prev, makeMessage("user", text)]);
      setImagining(true);
      const t0 = Date.now();
      try {
        const items = await grokApi.generateImage(text, imageCount, { aspectRatio });
        if (items.length === 0) throw new Error("生图返回空结果");
        setMessages((prev) => [
          ...prev,
          {
            id: nextId++,
            role: "assistant" as const,
            content: `已生成 ${items.length} 张图片`,
            images: items,
            durationMs: Date.now() - t0,
          },
        ]);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
        setMessages((prev) => [
          ...prev,
          {
            id: nextId++,
            role: "assistant" as const,
            content: msg,
            error: true,
            durationMs: Date.now() - t0,
          },
        ]);
      } finally {
        setImagining(false);
        setMessages((prev) => {
          persistNow(prev);
          return prev;
        });
      }
    },
    [imageCount, aspectRatio, persistNow],
  );

  const send = useCallback(async () => {
    const text = draft.trim();
    if (!text || sendingRef.current || imaginingRef.current) return;
    const attachments = [...pendingImages];
    if (attachments.length === 0 && looksLikeImaginePrompt(text)) {
      setDraft("");
      await runImagine(text);
      return;
    }
    setDraft("");
    setPendingImages([]);
    setError("");
    const userMsg: ChatMessage = { ...makeMessage("user", text), attachments };
    setMessages((prev) => [...prev, userMsg]);
    setSending(true);
    setMessages((prev) => [...prev, makeMessage("assistant", "")]);
    const t0 = Date.now();
    try {
      const history = [...messages, userMsg];
      await runCompletion(history);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      appendAssistant(msg, true);
    } finally {
      stampDuration(Date.now() - t0);
      setSending(false);
      setMessages((prev) => {
        persistNow(prev);
        return prev;
      });
    }
  }, [draft, messages, pendingImages, appendAssistant, persistNow, runCompletion, runImagine, stampDuration]);

  useEffect(() => {
    if (resendIndex == null || sendingRef.current) return;
    const target = messages[resendIndex];
    if (!target || target.role !== "user") {
      onResendDone?.();
      return;
    }
    const history = messages.slice(0, resendIndex + 1);
    setSending(true);
    setMessages([...history, makeMessage("assistant", "")]);
    const t0 = Date.now();
    void (async () => {
      try {
        await runCompletion(history);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
        appendAssistant(msg, true);
      } finally {
        stampDuration(Date.now() - t0);
        setSending(false);
        setMessages((prev) => {
          persistNow(prev);
          return prev;
        });
        onResendDone?.();
      }
    })();
  }, [resendIndex, messages, runCompletion, appendAssistant, persistNow, onResendDone, stampDuration]);

  const clear = useCallback(() => {
    if (sendingRef.current || imaginingRef.current) return;
    setMessages([]);
    setError("");
    persistNow([]);
  }, [persistNow]);

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
    if (!text) return;
    setDraft("");
    await runImagine(text);
  }, [draft, runImagine]);

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
            onChange={(e) => {
              const next = e.target.value as GrokChatModel;
              setModel(next);
              persistNow(messages, next);
            }}
            className="neo-input h-8 rounded-lg px-2 text-sm font-medium text-[var(--neo-ink)] focus-visible:outline-none"
            aria-label="Grok 模型"
          >
            {GROK_CHAT_MODELS.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
          {activeAccountId != null && (
            user?.role === "admin" ? (
              <a
                href="/grok/accounts/"
                className="rounded-full bg-[var(--neo-surface-muted)] px-2 py-0.5 text-xs text-[var(--neo-muted)] hover:text-[var(--neo-primary)] hover:underline"
              >
                账号 #{activeAccountId}
              </a>
            ) : (
              <span className="rounded-full bg-[var(--neo-surface-muted)] px-2 py-0.5 text-xs text-[var(--neo-muted)]">
                账号 #{activeAccountId}
              </span>
            )
          )}
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
              对话走纯 HTTP Rust 网关（<code className="rounded bg-[var(--neo-surface-muted)] px-1">/grok/v1/chat/completions</code>
              ）；上传图片自动走 OCR 链路（upload-file → grok-vision-ocr）。生图走{" "}
              <code className="rounded bg-[var(--neo-surface-muted)] px-1">/grok/v1/images/generations</code>。
            </p>
          </div>
        ) : (
          <div className="mx-auto flex max-w-3xl flex-col gap-4">
            {messages.map((m, i) => (
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
                    "max-w-[85%] break-words rounded-2xl px-4 py-2.5 text-sm leading-relaxed shadow-sm",
                    m.role === "user"
                      ? "whitespace-pre-wrap rounded-tr-sm bg-[var(--neo-primary)] text-white"
                      : cn(
                          "rounded-tl-sm border border-[var(--neo-border)] bg-white text-[var(--neo-ink)]",
                          m.error && "border-rose-200 bg-rose-50 text-rose-700",
                        ),
                  )}
                >
                  {m.role === "assistant" && !m.error ? (
                    m.content.trim() ? (
                      <ChatMessageContent content={m.content} role="assistant" />
                    ) : (
                      <span className="text-[var(--neo-muted)]">…</span>
                    )
                  ) : (
                    m.content || <span className="text-[var(--neo-muted)]">…</span>
                  )}
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
                  {m.role === "user" && onResendRequest && (
                    <button
                      type="button"
                      className="mt-1 block text-[11px] text-white/80 underline-offset-2 hover:underline"
                      onClick={() => onResendRequest(i)}
                      disabled={sending || imagining}
                    >
                      从此重发
                    </button>
                  )}
                  {m.role === "assistant" && m.durationMs != null && m.durationMs >= 0 && (
                    <p className="mt-1 text-[11px] text-[var(--neo-muted)]">
                      耗时 {formatReplyDuration(m.durationMs)}
                    </p>
                  )}
                </div>
              </div>
            ))}
            {sending && (
              <div className="flex items-center gap-2 pl-10 text-sm text-[var(--neo-muted)]">
                <Loader2 className="size-4 animate-spin" />
                生成中…{liveMs > 0 ? ` ${formatReplyDuration(liveMs)}` : ""}
              </div>
            )}
            {imagining && (
              <div className="flex items-center gap-2 pl-10 text-sm text-[var(--neo-muted)]">
                <Loader2 className="size-4 animate-spin" />
                正在生成图片…{liveMs > 0 ? ` ${formatReplyDuration(liveMs)}` : ""}
              </div>
            )}
          </div>
        )}
      </div>

      {/* 输入区 */}
      <div className="shrink-0 border-t border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-4 py-3">
        {pendingImages.length > 0 && (
          <div className="mx-auto mb-2 flex max-w-3xl flex-wrap gap-2">
            {pendingImages.map((url, i) => (
              // eslint-disable-next-line @next/next/no-img-element
              <img key={i} src={url} alt="" className="h-14 w-14 rounded-lg border object-cover" />
            ))}
          </div>
        )}
        <div className="mx-auto flex max-w-3xl items-end gap-2 rounded-2xl border border-[var(--neo-border)] bg-white p-2 shadow-sm">
          <input
            ref={fileRef}
            type="file"
            accept="image/*"
            className="hidden"
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (!file) return;
              const reader = new FileReader();
              reader.onload = () => {
                const url = String(reader.result || "");
                if (url.startsWith("data:")) setPendingImages((p) => [...p, url]);
              };
              reader.readAsDataURL(file);
              e.target.value = "";
            }}
          />
          <Button
            type="button"
            size="icon"
            variant="ghost"
            onClick={() => fileRef.current?.click()}
            disabled={sending || imagining}
            aria-label="上传图片"
          >
            <Paperclip className="size-4" />
          </Button>
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="输入消息，Enter 发送，Shift+Enter 换行"
            className="min-h-[44px] max-h-40 flex-1 resize-none border-none bg-transparent px-2 py-2 text-[15px] leading-relaxed shadow-none placeholder:text-[var(--neo-muted)] focus-visible:outline-none"
            disabled={sending}
          />
          <select
            value={aspectRatio}
            onChange={(e) => setAspectRatio(e.target.value)}
            className="neo-input h-9 rounded-lg px-1.5 text-sm text-[var(--neo-ink)] focus-visible:outline-none"
            aria-label="画幅比例"
            title="画幅比例"
            disabled={sending || imagining}
          >
            {["1:1", "16:9", "9:16", "4:3", "3:4"].map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
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
            size="sm"
            variant="outline"
            onClick={() => void imagine()}
            disabled={sending || imagining || !draft.trim()}
            aria-label="生成图片"
            title="调用 Grok Imagine（/v1/images/generations）"
          >
            {imagining ? <Loader2 className="size-4 animate-spin" /> : <ImageIcon className="size-4" />}
            生图
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
          输入「生成一张…图」或点「生图」走 Imagine；普通回车仍是文本对话。
          提取图片文字请用工作台 OCR。请求由 TNexus 代理转发，无需单独配密钥。
        </p>
      </div>
    </div>
  );
}
