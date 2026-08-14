"use client";

import { Badge } from "@/components/ui/badge";
import { GrokQuotaHeatstrip } from "@/components/grok/grok-quota-heatstrip";
import type { GrokAccountView, GrokQuotaWindow } from "@/lib/grok-admin";
import { cn } from "@/lib/utils";
import { labelAuthStatus } from "@/lib/grok-labels";

const AUTH_STATUS_VARIANT: Record<string, "success" | "warning" | "danger" | "muted" | "info"> = {
  active: "success",
  restricted: "warning",
  banned: "danger",
  reauth_required: "warning",
  reauthRequired: "warning",
  unknown: "muted",
};

const PROVIDER_LABEL: Record<string, string> = {
  grok_build: "Build 构建",
  grok_web: "Web 网页",
  grok_console: "Console 控制台",
};

function fmtTime(value: string | null | undefined): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

export type CooldownState =
  | { kind: "none" }
  | { kind: "past"; rawLabel: string }
  | { kind: "future"; label: string; rawLabel: string; remainingMs: number };

/** 将 cooldown_until 转为可展示状态；纯函数，方便单测。 */
export function formatCooldown(cooldownUntil: string | null, now = Date.now()): CooldownState {
  if (!cooldownUntil) return { kind: "none" };
  const d = new Date(cooldownUntil);
  if (Number.isNaN(d.getTime())) return { kind: "none" };
  const rawLabel = d.toLocaleString("zh-CN", { hour12: false });
  const remainingMs = d.getTime() - now;
  if (remainingMs <= 0) return { kind: "past", rawLabel };

  let label: string;
  const totalSec = Math.ceil(remainingMs / 1000);
  if (totalSec < 60) {
    label = `${totalSec}s`;
  } else if (totalSec < 3600) {
    label = `${Math.ceil(totalSec / 60)}min`;
  } else if (totalSec < 86400) {
    label = `${Math.ceil(totalSec / 3600)}h`;
  } else {
    label = `${Math.ceil(totalSec / 86400)}d`;
  }
  return { kind: "future", label, rawLabel, remainingMs };
}

function CooldownCell({ value }: { value: string | null }) {
  const state = formatCooldown(value);
  if (state.kind === "none") return <span className="text-[var(--neo-muted)]">—</span>;
  if (state.kind === "past") {
    return (
      <span className="text-xs text-[var(--neo-muted)]" title={state.rawLabel}>
        已恢复
      </span>
    );
  }
  return (
    <span className="text-xs" title={state.rawLabel}>
      冷却至 {state.rawLabel.slice(5, 16)}
      <span className="ml-1 text-amber-500">（{state.label}后）</span>
    </span>
  );
}

/** 冷却时间已过期的 last_error 视为历史残留。无冷却但有错误 = 当前故障。 */
export function isHistoricalError(cooldownUntil: string | null, now = Date.now()): boolean {
  return formatCooldown(cooldownUntil, now).kind === "past";
}

function LastErrorCell({ error, cooldownUntil }: { error: string | null; cooldownUntil: string | null }) {
  if (!error) return <span className="text-[var(--neo-muted)]">—</span>;
  const historical = isHistoricalError(cooldownUntil);
  return (
    <span
      className={historical ? "text-[var(--neo-muted)]" : "text-rose-600"}
      title={historical ? `历史残留（冷却已过）：${error}` : error}
    >
      {historical ? `历史：${error}` : error}
    </span>
  );
}

function authBadge(status: string) {
  const variant = AUTH_STATUS_VARIANT[status] ?? "muted";
  return <Badge variant={variant}>{labelAuthStatus(status)}</Badge>;
}

function providerLabel(provider: string) {
  return PROVIDER_LABEL[provider] ?? provider;
}

/** Grok 账号表格（只读；对齐 grok-admin `AccountView` 字段） */
export function GrokAccountsTable({
  items,
  onEdit,
  onDetail,
  quotaByAccount,
  quotaErrorByAccount,
  className,
}: {
  items: GrokAccountView[];
  /** 编辑按钮回调（打开编辑对话框）。 */
  onEdit?: (account: GrokAccountView) => void;
  /** 行点击回调（打开详情对话框）。 */
  onDetail?: (account: GrokAccountView) => void;
  /** 账号 → 额度窗口（GET /quota 的 `{items}` 解包后取 fast）。缺失显示「未知」。 */
  quotaByAccount?: Record<number, GrokQuotaWindow | null | undefined>;
  /** 账号 → 读取失败消息；有值时额度列渲染「读取失败」。 */
  quotaErrorByAccount?: Record<number, string>;
  className?: string;
}) {
  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
        <span>暂无账号数据</span>
        <span className="text-xs opacity-70">（grok-admin 返回空列表或未部署账号数据）</span>
      </div>
    );
  }

  return (
    <div className={cn("neo-card overflow-x-auto", className)}>
      <table className="w-full min-w-[940px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-3 py-2 font-medium">ID</th>
            <th className="px-3 py-2 font-medium">类型</th>
            <th className="px-3 py-2 font-medium">名称</th>
            <th className="px-3 py-2 font-medium">状态</th>
            <th className="px-3 py-2 font-medium">认证</th>
            <th className="px-3 py-2 font-medium">优先级</th>
            <th className="px-3 py-2 font-medium">已观测模型</th>
            <th className="px-3 py-2 font-medium">额度</th>
            <th className="px-3 py-2 font-medium">并发</th>
            <th className="px-3 py-2 font-medium">失败</th>
            <th className="px-3 py-2 font-medium">冷却至</th>
            <th className="px-3 py-2 font-medium">最近错误</th>
            <th className="px-3 py-2 font-medium">更新于</th>
            <th className="px-3 py-2 font-medium">操作</th>
          </tr>
        </thead>
        <tbody>
          {items.map((a) => (
            <tr
              key={a.id}
              className="border-b border-[var(--neo-border)] last:border-0 hover:bg-[var(--neo-surface-muted)]"
              onClick={() => onDetail?.(a)}
              style={onDetail ? { cursor: "pointer" } : undefined}
            >
              <td className="px-3 py-2 tabular-nums text-[var(--neo-muted)]">{a.id}</td>
              <td className="px-3 py-2">{providerLabel(a.provider)}</td>
              <td className="max-w-[160px] truncate px-3 py-2 font-medium text-[var(--neo-ink)]" title={a.name}>
                {a.name || "—"}
              </td>
              <td className="px-3 py-2">
                <Badge variant={a.enabled ? "success" : "muted"}>{a.enabled ? "启用" : "禁用"}</Badge>
              </td>
              <td className="px-3 py-2">{authBadge(a.auth_status)}</td>
              <td className="px-3 py-2 tabular-nums">{a.priority}</td>
              <td className="max-w-[180px] truncate px-3 py-2" title={a.observed_model ?? ""}>
                {a.observed_model || "—"}
              </td>
              <td className="px-3 py-2">
                {quotaErrorByAccount?.[a.id] ? (
                  <span
                    className="text-xs text-[var(--neo-muted)]"
                    title={quotaErrorByAccount[a.id]}
                  >
                    读取失败
                  </span>
                ) : (
                  <GrokQuotaHeatstrip window={quotaByAccount?.[a.id]} />
                )}
              </td>
              <td className="px-3 py-2 tabular-nums">{a.max_concurrent}</td>
              <td className="px-3 py-2 tabular-nums">{a.failure_count}</td>
              <td className="px-3 py-2 whitespace-nowrap">
                <CooldownCell value={a.cooldown_until} />
              </td>
              <td
                className="max-w-[200px] truncate px-3 py-2 text-xs"
              >
                <LastErrorCell error={a.last_error} cooldownUntil={a.cooldown_until} />
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(a.updated_at)}
              </td>
              <td className="px-3 py-2">
                {onEdit ? (
                  <button
                    type="button"
                    className="rounded-md border border-[var(--neo-border)] px-2 py-1 text-xs text-[var(--neo-muted)] hover:bg-stone-100 hover:text-[var(--neo-ink)]"
                    onClick={(e) => {
                      e.stopPropagation();
                      onEdit(a);
                    }}
                  >
                    编辑
                  </button>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
