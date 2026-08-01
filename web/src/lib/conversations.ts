import type { FactorPoint } from "@/lib/api";
import type { GenConfig } from "@/lib/gen-config";
import type { TextModelId } from "@/lib/models";

export type ConversationState = {
  prompt: string;
  mode: "director" | "casting";
  workflow: "full_agent" | "keyword_ps";
  enhanceEnabled: boolean;
  textModel: TextModelId;
  castingModels: TextModelId[];
  actorImageCounts: Record<TextModelId, number>;
  imageEngine: "chatgpt" | "grok";
  directorFactors: FactorPoint;
  renderFactors: FactorPoint;
  genConfig: GenConfig;
  activeAspect: string;
  lastJobId?: string | null;
};

export type Conversation = {
  id: string;
  title: string;
  state: ConversationState;
  created_at: string;
  updated_at: string;
};

export const EMPTY_CONVERSATION_STATE: ConversationState = {
  prompt: "",
  mode: "director",
  workflow: "full_agent",
  enhanceEnabled: false,
  textModel: "gpt",
  castingModels: ["gpt", "grok"],
  actorImageCounts: { gpt: 1, grok: 1, deepseek: 1, mimo: 1, hy3: 1 },
  imageEngine: "chatgpt",
  directorFactors: { x: 0.5, y: 0.5 },
  renderFactors: { x: 0.5, y: 0.5 },
  genConfig: {
    quality: "auto",
    width: 1024,
    height: 1024,
    count: 1,
    transparent_bg: false,
    align_16: true,
    polish_factor: 0,
  },
  activeAspect: "1:1",
  lastJobId: null,
};

export function conversationTitle(prompt: string): string {
  const t = prompt.trim();
  if (!t) return "新对话";
  return t.length > 24 ? `${t.slice(0, 24)}…` : t;
}
