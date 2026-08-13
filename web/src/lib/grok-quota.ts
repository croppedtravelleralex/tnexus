import type { GrokQuotaWindow } from "@/lib/grok-admin";

const MODE_PREF = ["fast", "auto", "console", "imagine"] as const;

/** grok-admin 列表接口包一层 `{ items }`；兼容误当成裸数组的旧客户端。 */
export function unwrapAdminItems<T>(raw: unknown): T[] {
  if (Array.isArray(raw)) return raw as T[];
  if (raw && typeof raw === "object" && Array.isArray((raw as { items?: unknown }).items)) {
    return (raw as { items: T[] }).items;
  }
  return [];
}

/** 号池热条：优先展示 rate-limits 写入的 fast 窗口。 */
export function pickDisplayQuotaWindow(
  windows: GrokQuotaWindow[] | null | undefined,
): GrokQuotaWindow | null {
  if (!windows || windows.length === 0) return null;
  for (const mode of MODE_PREF) {
    const hit = windows.find((w) => w.mode === mode && (w.total > 0 || w.remaining > 0));
    if (hit) return hit;
  }
  return windows.find((w) => w.total > 0 || w.remaining > 0) ?? windows[0] ?? null;
}
