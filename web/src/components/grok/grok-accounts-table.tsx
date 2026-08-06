"use client";

import { Badge } from "@/components/ui/badge";
import type { GrokAccountView } from "@/lib/grok-admin";
import { cn } from "@/lib/utils";

const AUTH_STATUS_VARIANT: Record<string, "success" | "warning" | "danger" | "muted" | "info"> = {
  active: "success",
  restricted: "warning",
  banned: "danger",
  reauth_required: "warning",
  reauthRequired: "warning",
  unknown: "muted",
};

const PROVIDER_LABEL: Record<string, string> = {
  grok_build: "Build",
  grok_web: "Web",
  grok_console: "Console",
};

function fmtTime(value: string | null | undefined): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function authBadge(status: string) {
  const variant = AUTH_STATUS_VARIANT[status] ?? "muted";
  const label = status.replace(/^reauth_?/, "reauth:");
  return <Badge variant={variant}>{label}</Badge>;
}

function providerLabel(provider: string) {
  return PROVIDER_LABEL[provider] ?? provider;
}

/** Grok 账号表格（只读；对齐 grok-admin `AccountView` 字段） */
export function GrokAccountsTable({
  items,
  className,
}: {
  items: GrokAccountView[];
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
      <table className="w-full min-w-[880px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-3 py-2 font-medium">ID</th>
            <th className="px-3 py-2 font-medium">Provider</th>
            <th className="px-3 py-2 font-medium">名称</th>
            <th className="px-3 py-2 font-medium">状态</th>
            <th className="px-3 py-2 font-medium">认证</th>
            <th className="px-3 py-2 font-medium">优先级</th>
            <th className="px-3 py-2 font-medium">已观测模型</th>
            <th className="px-3 py-2 font-medium">并发</th>
            <th className="px-3 py-2 font-medium">失败</th>
            <th className="px-3 py-2 font-medium">冷却至</th>
            <th className="px-3 py-2 font-medium">最近错误</th>
            <th className="px-3 py-2 font-medium">更新于</th>
          </tr>
        </thead>
        <tbody>
          {items.map((a) => (
            <tr
              key={a.id}
              className="border-b border-[var(--neo-border)] last:border-0 hover:bg-[var(--neo-surface-muted)]"
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
              <td className="px-3 py-2 tabular-nums">{a.max_concurrent}</td>
              <td className="px-3 py-2 tabular-nums">{a.failure_count}</td>
              <td className="px-3 py-2 whitespace-nowrap text-xs">{fmtTime(a.cooldown_until)}</td>
              <td
                className="max-w-[200px] truncate px-3 py-2 text-xs text-[var(--neo-muted)]"
                title={a.last_error ?? ""}
              >
                {a.last_error || "—"}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(a.updated_at)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
