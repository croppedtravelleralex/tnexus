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

export const EMPTY_CHAT_STATE: ChatConversationState = {
  kind: "chat",
  messages: [],
  model: "gpt-4o-mini",
  stream: true,
};

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
  }));
}

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
