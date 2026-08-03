export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  images?: string[];
  createdAt?: number;
  elapsedMs?: number;
};

export type ChatConversationState = {
  kind: "chat";
  messages: ChatMessage[];
  model: string;
  stream: boolean;
};

export const CHAT_ACTIVE_SESSION_KEY = "tnexus-chat-active-id";

/** 发给网关的内部通道 id（不在 UI 展示 OpenAI 型号名） */
export const CHAT_TEXT_CHANNEL = "gpt-4o-mini";

export const CHAT_CHANNEL_LABEL = "文本对话";
export const CHAT_CHANNEL_HINT = "走号池 ChatGPT 上游通道";

export const EMPTY_CHAT_STATE: ChatConversationState = {
  kind: "chat",
  messages: [],
  model: CHAT_TEXT_CHANNEL,
  stream: true,
};

export function isEmptyChatConversation(c: { title?: string; state?: unknown }): boolean {
  if (!isChatConversationState(c.state)) return true;
  const msgs = c.state.messages ?? [];
  return msgs.length === 0 && (c.title ?? "新对话") === "新对话";
}

export function createChatMessage(role: ChatMessage["role"], content: string): ChatMessage {
  return {
    id: crypto.randomUUID(),
    role,
    content,
    createdAt: Date.now(),
  };
}

export function normalizeChatMessages(raw: ChatMessage[]): ChatMessage[] {
  return raw.map((m, i) => ({
    ...m,
    id: m.id ?? `legacy-${i}-${m.role}`,
    createdAt: m.createdAt ?? 0,
    role: m.role === "assistant" ? "assistant" : "user",
  }));
}

/** Old sessions stored title from user text but omitted user rows in state.messages. */
export function repairLegacyMessages(messages: ChatMessage[], title?: string): ChatMessage[] {
  const normalized = normalizeChatMessages(messages);
  if (normalized.some((m) => m.role === "user")) return normalized;
  const t = String(title ?? "").trim();
  if (!t || t === "新对话") return normalized;
  return [createChatMessage("user", t), ...normalized];
}

export const CHAT_MODEL_HINTS: Record<string, string> = {
  [CHAT_TEXT_CHANNEL]: CHAT_CHANNEL_HINT,
  "gpt-image-2": "生图通道（生图模式自动使用）",
};

/** @deprecated UI 不再展示网关型号列表 */
export const DEFAULT_CHAT_MODELS = [CHAT_TEXT_CHANNEL, "gpt-image-2"] as const;

export function chatConversationTitle(messages: ChatMessage[]): string {
  const lastUser = [...messages].reverse().find((m) => m.role === "user");
  const t = lastUser?.content.trim() ?? "";
  if (!t) return "新对话";
  return t.length > 24 ? `${t.slice(0, 24)}…` : t;
}

export function isChatConversationState(state: unknown): state is ChatConversationState {
  if (!state || typeof state !== "object") return false;
  const s = state as Record<string, unknown>;
  return s.kind === "chat" && Array.isArray(s.messages);
}

export function exportChatAsText(messages: ChatMessage[]): string {
  const lines: string[] = [];
  for (const m of messages) {
    const who = m.role === "user" ? "用户" : "助手";
    const time =
      m.createdAt ? new Date(m.createdAt).toLocaleString() : "";
    const elapsed = m.elapsedMs ? ` (${m.elapsedMs}ms)` : "";
    lines.push(`[${who}] ${time}${elapsed}`);
    lines.push(m.content || "（空）");
    if (m.images?.length) {
      lines.push(`（含 ${m.images.length} 张图片）`);
    }
    lines.push("");
  }
  return lines.join("\n").trimEnd();
}

export function exportChatAsMarkdown(messages: ChatMessage[]): string {
  const lines: string[] = ["# 对话导出", ""];
  for (const m of messages) {
    const who = m.role === "user" ? "用户" : "助手";
    const meta: string[] = [];
    if (m.createdAt) meta.push(new Date(m.createdAt).toLocaleString());
    if (m.elapsedMs) meta.push(`${m.elapsedMs}ms`);
    lines.push(`## ${who}${meta.length ? ` · ${meta.join(" · ")}` : ""}`);
    lines.push("");
    lines.push(m.content || "（空）");
    if (m.images?.length) {
      lines.push("");
      lines.push(`*含 ${m.images.length} 张生成图（base64 未写入 md）*`);
    }
    lines.push("");
  }
  return lines.join("\n").trimEnd();
}

export function downloadTextFile(filename: string, content: string, mime = "text/plain;charset=utf-8") {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export function estimateBase64Bytes(b64: string): number {
  const padding = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((b64.length * 3) / 4) - padding);
}
