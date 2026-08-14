import type { GrokQuotaWindow } from "@/lib/grok-admin";
import { unwrapItems } from "@/lib/http";

/** 向后兼容别名：实现已移至 http.ts `unwrapItems`。 */
export { unwrapItems as unwrapAdminItems };

const MODE_PREF = ["fast", "auto", "console", "imagine"] as const;

/** 超过此时长未同步视为陈旧（与 heatstrip / summary 一致）。 */
export const QUOTA_STALE_MS = 24 * 60 * 60 * 1000;

/** 判断窗口是否超过 24h 未同步（或从未同步）。 */
export function isQuotaStale(window: Pick<GrokQuotaWindow, "synced_at">, now = Date.now()): boolean {
  if (!window.synced_at) return true;
  const syncedMs = new Date(window.synced_at).getTime();
  if (Number.isNaN(syncedMs)) return true;
  return now - syncedMs > QUOTA_STALE_MS;
}

function hasQuotaValue(window: Pick<GrokQuotaWindow, "remaining" | "total">): boolean {
  return window.total > 0 || window.remaining > 0;
}

/** 号池热条：优先新鲜 fast；没有新鲜窗口再回退陈旧值。 */
export function pickDisplayQuotaWindow(
  windows: GrokQuotaWindow[] | null | undefined,
  now = Date.now(),
): GrokQuotaWindow | null {
  if (!windows || windows.length === 0) return null;
  for (const mode of MODE_PREF) {
    const hit = windows.find(
      (w) => w.mode === mode && hasQuotaValue(w) && !isQuotaStale(w, now),
    );
    if (hit) return hit;
  }
  const anyFresh = windows.find((w) => hasQuotaValue(w) && !isQuotaStale(w, now));
  if (anyFresh) return anyFresh;
  for (const mode of MODE_PREF) {
    const hit = windows.find((w) => w.mode === mode && hasQuotaValue(w));
    if (hit) return hit;
  }
  return windows.find((w) => hasQuotaValue(w)) ?? windows[0] ?? null;
}
