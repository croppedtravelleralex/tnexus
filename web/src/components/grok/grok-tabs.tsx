"use client";

import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";

/** grok 管理子页导航（/grok/* 页内 tabs，避免顶栏膨胀）。 */
const GROK_TABS = [
  { href: "/grok/accounts/", label: "账号" },
  { href: "/grok/models/", label: "模型" },
  { href: "/grok/keys/", label: "密钥" },
  { href: "/grok/audits/", label: "审计" },
  { href: "/grok/dashboard/", label: "总览" },
  { href: "/grok/settings/", label: "设置" },
] as const;

function normPath(pathname: string) {
  return pathname.endsWith("/") ? pathname : `${pathname}/`;
}

/** 静态导出站点用原生 <a> 整页跳转。 */
export function GrokTabs({ className }: { className?: string }) {
  const pathname = usePathname() ?? "";
  const current = normPath(pathname);
  return (
    <nav
      className={cn(
        "mb-3 flex flex-wrap items-center gap-1 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface)] p-1",
        className,
      )}
      aria-label="Grok 管理子页"
    >
      {GROK_TABS.map((tab) => {
        const active = current === tab.href || current.startsWith(tab.href);
        return (
          <a
            key={tab.href}
            href={tab.href}
            className={cn(
              "rounded-md px-3 py-1.5 text-[13px] font-medium shadow-none transition",
              active
                ? "bg-white text-[var(--neo-ink)] ring-1 ring-[var(--neo-border)]"
                : "text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)] hover:text-[var(--neo-ink)]",
            )}
          >
            {tab.label}
          </a>
        );
      })}
    </nav>
  );
}
