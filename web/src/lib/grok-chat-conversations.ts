import type { GrokChatModel } from "@/components/grok-chat/grok-chat-panel";

export type GrokChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  error?: boolean;
  images?: string[];
  attachments?: string[];
  createdAt?: number;
};

export type GrokChatConversationState = {
  kind: "grok-chat";
  messages: GrokChatMessage[];
  model: GrokChatModel;
  /** 最近一次对话使用的号池账号 */
  lastAccountId?: number | null;
};

export const GROK_CHAT_ACTIVE_SESSION_KEY = "tnexus-grok-chat-active-id";

export const EMPTY_GROK_CHAT_STATE: GrokChatConversationState = {
  kind: "grok-chat",
  messages: [],
  model: "grok-chat-fast",
};

export function isGrokChatConversationState(state: unknown): state is GrokChatConversationState {
  return (
    typeof state === "object" &&
    state !== null &&
    (state as GrokChatConversationState).kind === "grok-chat" &&
    Array.isArray((state as GrokChatConversationState).messages)
  );
}

export function isEmptyGrokChatConversation(c: { title?: string; state?: unknown }): boolean {
  if (!isGrokChatConversationState(c.state)) return true;
  const msgs = c.state.messages ?? [];
  return msgs.length === 0 && (c.title ?? "新对话") === "新对话";
}

export function createGrokChatMessage(
  role: GrokChatMessage["role"],
  content: string,
): GrokChatMessage {
  return {
    id: crypto.randomUUID(),
    role,
    content,
    createdAt: Date.now(),
  };
}

export function grokChatExportMarkdown(messages: GrokChatMessage[]): string {
  return messages
    .map((m) => `### ${m.role === "user" ? "用户" : "助手"}\n\n${m.content}`)
    .join("\n\n---\n\n");
}

export function grokChatExportText(messages: GrokChatMessage[]): string {
  return messages.map((m) => `${m.role}: ${m.content}`).join("\n\n");
}

export function grokChatConversationTitle(messages: GrokChatMessage[]): string {
  const lastUser = [...messages].reverse().find((m) => m.role === "user");
  const t = lastUser?.content.trim() ?? "";
  if (!t) return "新对话";
  return t.length > 24 ? `${t.slice(0, 24)}…` : t;
}
