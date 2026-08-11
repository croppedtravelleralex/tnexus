"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Download, LoaderCircle, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { conversationsApi } from "@/lib/api";
import type { Conversation } from "@/lib/conversations";
import { GrokChatPanel } from "@/components/grok-chat/grok-chat-panel";
import {
  EMPTY_GROK_CHAT_STATE,
  GROK_CHAT_ACTIVE_SESSION_KEY,
  grokChatConversationTitle,
  grokChatExportMarkdown,
  grokChatExportText,
  isEmptyGrokChatConversation,
  isGrokChatConversationState,
  type GrokChatConversationState,
  type GrokChatMessage,
} from "@/lib/grok-chat-conversations";
import { cn } from "@/lib/utils";

function toPanelMessages(messages: GrokChatMessage[]) {
  return messages.map((m) => ({
    id: m.id,
    role: m.role,
    content: m.content,
    error: m.error,
    images: m.images,
    attachments: m.attachments,
  }));
}

export function GrokChatWorkbench() {
  const [conversationId, setConversationId] = useState<string | null>(null);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [panelState, setPanelState] = useState<GrokChatConversationState>(EMPTY_GROK_CHAT_STATE);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [resendIndex, setResendIndex] = useState<number | null>(null);
  const bootedRef = useRef(false);
  const cacheRef = useRef<Map<string, Conversation>>(new Map());

  const mergeConversation = useCallback((c: Conversation) => {
    cacheRef.current.set(c.id, c);
    setConversations((prev) => {
      const idx = prev.findIndex((item) => item.id === c.id);
      const next = idx >= 0 ? [...prev] : [c, ...prev];
      if (idx >= 0) next[idx] = c;
      return next.sort(
        (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
      );
    });
    return c;
  }, []);

  const visibleConversations = useMemo(() => {
    return conversations.filter((c) => {
      if (c.id === conversationId) return true;
      return !isEmptyGrokChatConversation(c);
    });
  }, [conversations, conversationId]);

  const loadConversations = useCallback(async () => {
    const list = await conversationsApi.list();
    const grokOnly = list.filter((c) => isGrokChatConversationState(c.state));
    for (const c of grokOnly) {
      cacheRef.current.set(c.id, c as Conversation);
    }
    setConversations(grokOnly as Conversation[]);
    return grokOnly;
  }, []);

  const applyConversation = useCallback((c: Conversation) => {
    if (!isGrokChatConversationState(c.state)) return;
    setConversationId(c.id);
    sessionStorage.setItem(GROK_CHAT_ACTIVE_SESSION_KEY, c.id);
    setPanelState(c.state);
    setError("");
  }, []);

  const persistState = useCallback(
    async (id: string, state: GrokChatConversationState) => {
      const patched = await conversationsApi.patch(id, {
        title: grokChatConversationTitle(state.messages),
        state,
      });
      mergeConversation(patched);
      return patched;
    },
    [mergeConversation],
  );

  const ensureConversation = useCallback(async () => {
    if (conversationId) return conversationId;
    const created = await conversationsApi.create({
      title: "新对话",
      state: EMPTY_GROK_CHAT_STATE,
    });
    mergeConversation(created);
    setConversationId(created.id);
    sessionStorage.setItem(GROK_CHAT_ACTIVE_SESSION_KEY, created.id);
    return created.id;
  }, [conversationId, mergeConversation]);

  const selectConversation = useCallback(
    async (id: string) => {
      if (id === conversationId && !switchingId) return;
      setSwitchingId(id);
      setError("");
      const cached = cacheRef.current.get(id) ?? conversations.find((c) => c.id === id);
      if (cached && isGrokChatConversationState(cached.state)) {
        applyConversation(cached);
      }
      try {
        const c = await conversationsApi.get(id);
        mergeConversation(c);
        applyConversation(c);
      } catch (err) {
        setError(err instanceof Error ? err.message : "加载对话失败");
      } finally {
        setSwitchingId(null);
      }
    },
    [applyConversation, conversationId, conversations, mergeConversation, switchingId],
  );

  const startNewConversation = useCallback(() => {
    setConversationId(null);
    setPanelState(EMPTY_GROK_CHAT_STATE);
    setError("");
    sessionStorage.removeItem(GROK_CHAT_ACTIVE_SESSION_KEY);
  }, []);

  const deleteConversation = useCallback(
    async (id: string) => {
      await conversationsApi.delete(id);
      cacheRef.current.delete(id);
      setConversations((prev) => prev.filter((c) => c.id !== id));
      if (conversationId === id) {
        startNewConversation();
      }
    },
    [conversationId, startNewConversation],
  );

  useEffect(() => {
    if (bootedRef.current) return;
    bootedRef.current = true;
    void (async () => {
      const list = await loadConversations();
      const stored = sessionStorage.getItem(GROK_CHAT_ACTIVE_SESSION_KEY);
      if (stored) {
        try {
          const c = await conversationsApi.get(stored);
          if (isGrokChatConversationState(c.state)) {
            applyConversation(c);
            return;
          }
        } catch {
          sessionStorage.removeItem(GROK_CHAT_ACTIVE_SESSION_KEY);
        }
      }
      if (list.length > 0 && isGrokChatConversationState(list[0].state)) {
        applyConversation(list[0] as Conversation);
      }
    })();
  }, [applyConversation, loadConversations]);

  const handlePersist = useCallback(
    async (state: {
      model: GrokChatConversationState["model"];
      messages: Array<{ id: number; role: "user" | "assistant"; content: string; error?: boolean; images?: string[]; attachments?: string[] }>;
      lastAccountId?: number | null;
    }) => {
      const next: GrokChatConversationState = {
        kind: "grok-chat",
        model: state.model,
        lastAccountId: state.lastAccountId ?? panelState.lastAccountId ?? null,
        messages: state.messages.map((m) => ({
          id: String(m.id),
          role: m.role,
          content: m.content,
          error: m.error,
          images: m.images,
          attachments: m.attachments,
          createdAt: Date.now(),
        })),
      };
      setPanelState(next);
      try {
        const id = await ensureConversation();
        await persistState(id, next);
      } catch (err) {
        setError(err instanceof Error ? err.message : "保存对话失败");
      }
    },
    [ensureConversation, persistState, panelState.lastAccountId],
  );

  const downloadExport = useCallback(
    (format: "md" | "txt") => {
      const msgs = panelState.messages;
      if (msgs.length === 0) return;
      const title = grokChatConversationTitle(msgs).replace(/[^\w\u4e00-\u9fa5-]+/g, "_");
      const body = format === "md" ? grokChatExportMarkdown(msgs) : grokChatExportText(msgs);
      const blob = new Blob([body], {
        type: format === "md" ? "text/markdown;charset=utf-8" : "text/plain;charset=utf-8",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${title || "grok-chat"}.${format === "md" ? "md" : "txt"}`;
      a.click();
      URL.revokeObjectURL(url);
    },
    [panelState.messages],
  );

  return (
    <div className="flex h-full min-h-0">
      <aside
        className={cn(
          "flex shrink-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface-muted)] transition-[width]",
          sidebarOpen ? "w-56 sm:w-64" : "w-0 overflow-hidden border-0",
        )}
      >
        <div className="flex items-center justify-between gap-2 border-b border-[var(--neo-border)] px-3 py-2">
          <span className="text-xs font-semibold uppercase tracking-wide text-[var(--neo-muted)]">对话</span>
          <Button variant="ghost" size="sm" className="h-7 px-2" onClick={startNewConversation}>
            <Plus className="size-4" />
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {visibleConversations.length === 0 ? (
            <p className="px-2 py-4 text-xs text-[var(--neo-muted)]">暂无历史，发送消息将自动创建</p>
          ) : (
            <ul className="space-y-1">
              {visibleConversations.map((c) => (
                <li key={c.id}>
                  <div
                    className={cn(
                      "group flex w-full items-center gap-1 rounded-lg px-2 py-2 text-sm",
                      conversationId === c.id
                        ? "bg-white text-[var(--neo-ink)] shadow-sm"
                        : "text-[var(--neo-muted)] hover:bg-white/70 hover:text-[var(--neo-ink)]",
                    )}
                  >
                    <button
                      type="button"
                      className="min-w-0 flex-1 truncate text-left"
                      onClick={() => void selectConversation(c.id)}
                    >
                      {c.title || "新对话"}
                    </button>
                    {switchingId === c.id ? (
                      <LoaderCircle className="size-3.5 shrink-0 animate-spin" />
                    ) : (
                      <button
                        type="button"
                        className="hidden shrink-0 rounded p-0.5 group-hover:inline-flex hover:bg-rose-50 hover:text-rose-600"
                        onClick={() => void deleteConversation(c.id)}
                        aria-label="删除对话"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </aside>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center gap-2 border-b border-[var(--neo-border)] px-2 py-1">
          <Button variant="ghost" size="sm" onClick={() => setSidebarOpen((v) => !v)}>
            {sidebarOpen ? <ChevronLeft className="size-4" /> : <ChevronRight className="size-4" />}
          </Button>
          {panelState.lastAccountId != null && (
            <span className="text-xs text-[var(--neo-muted)]">调度账号 #{panelState.lastAccountId}</span>
          )}
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              disabled={panelState.messages.length === 0}
              onClick={() => downloadExport("md")}
            >
              <Download className="size-4" />
              MD
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={panelState.messages.length === 0}
              onClick={() => downloadExport("txt")}
            >
              TXT
            </Button>
          </div>
          {error ? <p className="truncate text-xs text-rose-600">{error}</p> : null}
        </div>
        <GrokChatPanel
          sessionKey={conversationId}
          initialModel={panelState.model}
          initialMessages={toPanelMessages(panelState.messages)}
          initialLastAccountId={panelState.lastAccountId ?? null}
          resendIndex={resendIndex}
          onResendDone={() => setResendIndex(null)}
          onResendRequest={(index) => setResendIndex(index)}
          onPersist={(s) => void handlePersist(s)}
        />
      </div>
    </div>
  );
}
