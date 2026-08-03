export type ChatMessage = {
  role: "user" | "assistant";
  content: string;
  images?: string[];
};

export type ChatConversationState = {
  kind: "chat";
  messages: ChatMessage[];
  model: string;
  stream: boolean;
};

export const EMPTY_CHAT_STATE: ChatConversationState = {
  kind: "chat",
  messages: [],
  model: "gpt-4o-mini",
  stream: true,
};

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
