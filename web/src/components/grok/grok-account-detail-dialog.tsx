"use client";

import { LoaderCircle, X } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { grokAdminApi, type GrokAccountView, type GrokAccountDetail, type GrokQuotaWindow } from "@/lib/grok-admin";
import { labelModelStatusLine, labelQuotaMode, labelQuotaSource } from "@/lib/grok-labels";
import { isQuotaStale } from "@/lib/grok-quota";
import { QUOTA_UNLIMITED_THRESHOLD } from "@/components/grok/grok-quota-heatstrip";

type Props = {
  open: boolean;
  account: GrokAccountView | null;
  token: string;
  onOpenChange: (open: boolean) => void;
  /** 从详情进入编辑（account 复用当前行数据）。 */
  onEdit: (account: GrokAccountView) => void;
};

const MODEL_STATUS_VARIANT: Record<string, string> = {
  available: "text-emerald-600",
  quota_available: "text-emerald-600",
  unknown: "text-stone-400",
  soft_stop: "text-amber-600",
  expired_soft_stop: "text-stone-400",
  quota_exhausted: "text-rose-600",
  auth_failed: "text-rose-600",
  signature_failed: "text-rose-600",
};

function fmtTime(value: string | null | undefined): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function isSoftStopCurrent(cooldownUntil: string | null | undefined, updatedAt: string, now = Date.now()): boolean {
  if (cooldownUntil) {
    const t = new Date(cooldownUntil).getTime();
    return !Number.isNaN(t) && t > now;
  }
  const upd = new Date(updatedAt).getTime();
  if (Number.isNaN(upd)) return false;
  return now - upd < 24 * 60 * 60 * 1000;
}

function quotaRemainingLabel(w: GrokQuotaWindow): string {
  if (w.total >= QUOTA_UNLIMITED_THRESHOLD) return "不限";
  if (w.total === 0 && w.remaining === 0) return "未知";
  return `${w.remaining} / ${w.total}`;
}

/** 账号详情：额度窗口 + 模型状态（GET /admin/accounts/:id）。 */
export function GrokAccountDetailDialog({ open, account, token, onOpenChange, onEdit }: Props) {
  const [detail, setDetail] = useState<GrokAccountDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open || !account) return;
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      setLoading(true);
      setError("");
      setDetail(null);
    });
    grokAdminApi
      .getDetail(token, account.id)
      .then((data) => {
        if (!cancelled) setDetail(data);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, account, token]);

  if (!open || !account) return null;

  const windows = detail?.quota_windows ?? [];
  const states = detail?.model_states ?? [];

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card max-h-[85vh] w-full max-w-2xl overflow-y-auto p-6">
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">账号详情</h2>
            <p className="mt-1 truncate text-sm text-[var(--neo-muted)]">
              #{account.id} · {account.name || "—"} · {account.provider}
            </p>
          </div>
          <button
            type="button"
            className="rounded-lg p-1 text-[var(--neo-muted)] hover:bg-stone-100"
            onClick={() => onOpenChange(false)}
            aria-label="关闭"
          >
            <X className="size-5" />
          </button>
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 py-10 text-sm text-[var(--neo-muted)]">
            <LoaderCircle className="size-4 animate-spin" /> 加载中…
          </div>
        ) : error ? (
          <p className="py-6 text-center text-sm text-rose-600">{error}</p>
        ) : (
          <div className="space-y-6">
            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--neo-muted)]">
                额度窗口（{windows.length}）
              </h3>
              {windows.length === 0 ? (
                <p className="text-sm text-[var(--neo-muted)]">无额度窗口记录</p>
              ) : (
                <div className="overflow-x-auto rounded-lg border border-[var(--neo-border)]">
                  <table className="w-full min-w-[420px] border-collapse text-left text-sm">
                    <thead>
                      <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
                        <th className="px-3 py-2 font-medium">模式</th>
                        <th className="px-3 py-2 font-medium">剩余/总额</th>
                        <th className="px-3 py-2 font-medium">重置</th>
                        <th className="px-3 py-2 font-medium">来源</th>
                        <th className="px-3 py-2 font-medium">同步于</th>
                      </tr>
                    </thead>
                    <tbody>
                      {windows.map((w) => {
                        const stale = isQuotaStale(w);
                        const unknown = w.total === 0 && w.remaining === 0;
                        return (
                          <tr key={w.mode} className="border-b border-[var(--neo-border)] last:border-0">
                            <td className="px-3 py-2 font-medium text-[var(--neo-ink)]">
                              {labelQuotaMode(w.mode)}
                              {stale ? (
                                <span className="ml-1 text-[10px] font-normal text-amber-600">陈旧</span>
                              ) : null}
                            </td>
                            <td className="px-3 py-2 tabular-nums">
                              <span
                                className={
                                  unknown || stale
                                    ? "text-[var(--neo-muted)]"
                                    : w.total > 0 && w.remaining <= 0
                                      ? "text-rose-600"
                                      : ""
                                }
                              >
                                {quotaRemainingLabel(w)}
                              </span>
                            </td>
                            <td className="px-3 py-2 whitespace-nowrap text-xs">{fmtTime(w.reset_at)}</td>
                            <td className="px-3 py-2">{labelQuotaSource(w.source)}</td>
                            <td className="px-3 py-2 whitespace-nowrap text-xs">{fmtTime(w.synced_at)}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-[var(--neo-muted)]">
                模型状态（{states.length}）
              </h3>
              {states.length === 0 ? (
                <p className="text-sm text-[var(--neo-muted)]">无模型状态记录</p>
              ) : (
                <div className="space-y-2">
                  {states.map((s) => {
                    const currentSoftStop =
                      s.status === "soft_stop" && isSoftStopCurrent(s.cooldown_until, s.updated_at);
                    const statusLabel = currentSoftStop
                      ? labelModelStatusLine(s.status, s.reason)
                      : s.status === "soft_stop"
                        ? labelModelStatusLine("unknown", "expired_soft_stop")
                        : labelModelStatusLine(s.status, s.reason);
                    const statusClass = currentSoftStop
                      ? MODEL_STATUS_VARIANT.soft_stop
                      : s.status === "soft_stop"
                        ? MODEL_STATUS_VARIANT.expired_soft_stop
                        : (MODEL_STATUS_VARIANT[s.status] ?? "text-stone-500");
                    return (
                      <div
                        key={s.upstream_model}
                        className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[var(--neo-border)] px-3 py-2 text-sm"
                      >
                        <span className="font-medium text-[var(--neo-ink)]">{s.upstream_model}</span>
                        <span className={statusClass}>{statusLabel}</span>
                        <span className="text-xs text-[var(--neo-muted)]">
                          失败 {s.consecutive_failures} · 冷却至 {fmtTime(s.cooldown_until)}
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          </div>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
            关闭
          </Button>
          <Button size="sm" onClick={() => onEdit(account)}>
            编辑账号
          </Button>
        </div>
      </div>
    </div>
  );
}