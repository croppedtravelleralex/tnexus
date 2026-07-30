export type TextModelId = "gpt" | "grok" | "deepseek" | "mimo" | "hy3";

export const TEXT_MODELS: { id: TextModelId; label: string; description: string }[] = [
  { id: "gpt", label: "GPT", description: "OpenAI 系列" },
  { id: "grok", label: "Grok", description: "xAI 系列" },
  { id: "deepseek", label: "DeepSeek", description: "深度求索" },
  { id: "mimo", label: "Mimo", description: "小米 MiMo" },
  { id: "hy3", label: "HY3", description: "混元 3" },
];

export const IMAGE_ENGINES = [
  { id: "chatgpt" as const, label: "ChatGPT 绘图" },
  { id: "grok" as const, label: "Grok 绘图" },
];

export function textModelLabel(id: string): string {
  return TEXT_MODELS.find((m) => m.id === id)?.label ?? id;
}
