"use client";

import Link from "next/link";
import Image from "next/image";
import { usePathname, useRouter } from "next/navigation";
import { Menu } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useAuth } from "@/lib/auth";
import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { href: "/studio", label: "TNexus", studioHome: true },
  { href: "/accounts", label: "号池管理" },
  { href: "/image-manager", label: "图片管理" },
  { href: "/logs", label: "日志管理" },
  { href: "/ops", label: "运维" },
  { href: "/chat", label: "对话" },
  { href: "/settings", label: "设置" },
] as const;

function isNavActive(pathname: string, href: string, studioHome?: boolean) {
  if (studioHome) {
    return pathname === "/studio" || pathname.startsWith("/history");
  }
  return pathname === href || pathname.startsWith(`${href}/`);
}

function navLinkClass(active: boolean, studioItem?: boolean) {
  return cn(
    "relative shrink-0 whitespace-nowrap px-1 py-1 text-[13px] font-medium shadow-none transition sm:text-sm",
    active
      ? "font-semibold text-[var(--neo-ink)]"
      : "text-[var(--neo-muted)] hover:text-[var(--neo-ink)]",
    studioItem && !active && "text-[var(--neo-ink)]",
  );
}

export function TopNav() {
  const pathname = usePathname();
  const router = useRouter();
  const { user, bootstrapping, logout } = useAuth();
  const [menuOpen, setMenuOpen] = useState(false);

  if (pathname === "/login" || pathname === "/register") {
    return null;
  }

  const roleLabel = user?.role === "admin" ? "管理员" : "用户";
  const displayName = user?.display_name || user?.email || "";

  return (
    <header className="sticky top-0 z-50 border-b border-[var(--neo-border)] bg-white/95">
      <div className="mx-auto flex h-12 max-w-[1600px] items-center gap-3 px-4 sm:px-6">
        <button
          type="button"
          className="inline-flex size-8 items-center justify-center rounded-md text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)] sm:hidden"
          onClick={() => setMenuOpen((v) => !v)}
          aria-label="打开菜单"
        >
          <Menu className="size-4" />
        </button>

        <Link href="/studio" prefetch className="flex shrink-0 items-center gap-2" aria-label="TNexus 工作台">
          <Image src="/logo.png" alt="TNexus" width={28} height={28} className="rounded-md" />
          <span className="text-[15px] font-bold tracking-tight text-[var(--neo-ink)]">TNexus</span>
        </Link>

        <nav className="hidden min-w-0 flex-1 items-center justify-center gap-1 sm:flex sm:gap-6">
          {NAV_ITEMS.map((item) => {
            const active = isNavActive(pathname, item.href, "studioHome" in item ? item.studioHome : false);
            return (
              <Link
                key={item.href}
                href={item.href}
                prefetch
                className={navLinkClass(active, item.href === "/studio")}
              >
                {item.label}
                {active ? (
                  <span className="absolute inset-x-0 -bottom-[13px] hidden h-0.5 rounded-full bg-[var(--neo-primary-deep)] sm:block" />
                ) : null}
              </Link>
            );
          })}
        </nav>

        <div className="ml-auto flex items-center gap-2">
          {!bootstrapping && user ? (
            <>
              <span className="hidden rounded-md bg-[var(--neo-surface-muted)] px-2 py-1 text-[11px] font-medium text-[var(--neo-muted)] sm:inline">
                {roleLabel} · {displayName}
              </span>
              <Button size="sm" variant="ghost" className="h-8 shadow-none" onClick={() => void logout().then(() => router.push("/login"))}>
                退出
              </Button>
            </>
          ) : (
            <Link href="/login" prefetch>
              <Button size="sm">登录</Button>
            </Link>
          )}
        </div>
      </div>

      {menuOpen ? (
        <nav className="border-t border-[var(--neo-border)] bg-white px-4 py-2 sm:hidden">
          {NAV_ITEMS.map((item) => {
            const active = isNavActive(pathname, item.href, "studioHome" in item ? item.studioHome : false);
            return (
              <Link
                key={item.href}
                href={item.href}
                prefetch
                onClick={() => setMenuOpen(false)}
                className={cn(
                  "mb-0.5 block w-full rounded-lg px-3 py-2 text-left text-sm shadow-none",
                  active
                    ? "bg-[var(--neo-surface-muted)] font-semibold text-[var(--neo-ink)]"
                    : "text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)]",
                )}
              >
                {item.label}
              </Link>
            );
          })}
        </nav>
      ) : null}
    </header>
  );
}
