import type { GrokQuotaWindow } from "@/lib/grok-admin";
import { unwrapItems } from "@/lib/http";

/** 向后兼容别名：实现已移至 http.ts `unwrapItems`。 */
export { unwrapItems as unwrapAdminItems };

const MODE_PREF = ["fast", "auto", "console", "imagine"] as const;

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
