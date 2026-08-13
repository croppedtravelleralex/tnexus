"use client";

import { Fragment, useState } from "react";
import Image from "next/image";
import { usePathname, useRouter } from "next/navigation";
import { Menu } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAuth } from "@/lib/auth";
import { cn } from "@/lib/utils";
import {
  NAV_ENTRIES,
  NAV_AREA_LABELS,
  NAV_AREA_ORDER,
  filterNavForRole,
  isNavActive,
} from "@/lib/nav";

function navLinkClass(active: boolean, studioItem?: boolean) {
  return cn(
    "relative shrink-0 whitespace-nowrap px-1 py-1 text-[13px] font-medium shadow-none transition sm:text-sm",
    active
      ? "font-semibold text-[var(--neo-ink)]"
      : "text-[var(--neo-muted)] hover:text-[var(--neo-ink)]",
    studioItem && !active && "text-[var(--neo-ink)]",
  );
}

/** 静态导出站点用原生 <a> 整页跳转，避免客户端路由在部分环境下卡死 */
function NavAnchor({
  href,
  className,
  onNavigate,
  children,
  ...rest
}: {
  href: string;
  className?: string;
  onNavigate?: () => void;
  children: React.ReactNode;
} & Omit<
  React.AnchorHTMLAttributes<HTMLAnchorElement>,
  "href" | "className" | "onClick" | "children"
>) {
  return (
    <a
      href={href}
      className={className}
      onClick={() => onNavigate?.()}
      {...rest}
    >
      {children}
    </a>
  );
}

export function TopNav() {
  const pathname = usePathname();
  const router = useRouter();
  const { user, bootstrapping, logout } = useAuth();
  const [menuOpen, setMenuOpen] = useState(false);

  if (
    pathname === "/login" ||
    pathname === "/login/" ||
    pathname === "/register" ||
    pathname === "/register/"
  ) {
    return null;
  }

  const isAdmin = user?.role === "admin";
  const roleLabel = isAdmin ? "管理员" : "用户";
  const displayName = user?.display_name || user?.email || "";

  // bootstrapping 为 true 时 user 为 null（ConsoleLayout 已拦截），此处 isAdmin 安全
  const visibleEntries = filterNavForRole(NAV_ENTRIES, isAdmin);
  const groups = NAV_AREA_ORDER.map((area) => ({
    area,
    label: NAV_AREA_LABELS[area],
    items: visibleEntries.filter((e) => e.area === area),
  })).filter((g) => g.items.length > 0);

  return (
    <header className="sticky top-0 z-[100] border-b border-[var(--neo-border)] bg-white/95 backdrop-blur-sm">
      <div className="mx-auto flex h-12 max-w-[1600px] items-center gap-3 px-4 sm:px-6">
        <button
          type="button"
          className="inline-flex size-8 items-center justify-center rounded-md text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)] sm:hidden"
          onClick={() => setMenuOpen((v) => !v)}
          aria-label="打开菜单"
        >
          <Menu className="size-4" />
        </button>

        <NavAnchor
          href="/studio/"
          className="flex shrink-0 items-center gap-2"
          aria-label="TNexus 工作台"
        >
          <Image src="/logo.png" alt="TNexus" width={28} height={28} className="rounded-md" />
          <span className="text-[15px] font-bold tracking-tight text-[var(--neo-ink)]">TNexus</span>
        </NavAnchor>

        {/* 桌面导航：各区域间用细竖线分隔 */}
        <nav
          className="hidden min-w-0 flex-1 items-center justify-center sm:flex"
          aria-label="主导航"
        >
          {groups.map((group, gIdx) => (
            <Fragment key={group.area}>
              {gIdx > 0 && (
                <span
                  className="mx-3 h-4 w-px shrink-0 bg-[var(--neo-border)]"
                  aria-hidden="true"
                />
              )}
              <div className="flex items-center gap-4">
                {group.items.map((item) => {
                  const active = isNavActive(pathname, item);
                  return (
                    <NavAnchor
                      key={item.href}
                      href={item.href}
                      className={navLinkClass(active, item.href === "/studio/")}
                    >
                      {item.label}
                      {active ? (
                        <span className="absolute inset-x-0 -bottom-[13px] hidden h-0.5 rounded-full bg-[var(--neo-primary-deep)] sm:block" />
                      ) : null}
                    </NavAnchor>
                  );
                })}
              </div>
            </Fragment>
          ))}
        </nav>

        <div className="ml-auto flex items-center gap-2">
          {!bootstrapping && user ? (
            <>
              <span className="hidden rounded-md bg-[var(--neo-surface-muted)] px-2 py-1 text-[11px] font-medium text-[var(--neo-muted)] sm:inline">
                {roleLabel} · {displayName}
              </span>
              <Button
                size="sm"
                variant="ghost"
                className="h-8 shadow-none"
                onClick={() => void logout().then(() => router.push("/login/"))}
              >
                退出
              </Button>
            </>
          ) : (
            <NavAnchor href="/login/">
              <Button size="sm">登录</Button>
            </NavAnchor>
          )}
        </div>
      </div>

      {/* 移动端菜单：各区域显示分组标签 */}
      {menuOpen ? (
        <nav
          className="border-t border-[var(--neo-border)] bg-white px-4 py-2 sm:hidden"
          aria-label="移动端导航"
        >
          {groups.map((group, gIdx) => (
            <div
              key={group.area}
              className={
                gIdx > 0
                  ? "mt-1.5 border-t border-[var(--neo-border)] pt-1.5"
                  : ""
              }
            >
              <div className="mb-1 px-3 text-[10px] font-semibold uppercase tracking-wider text-[var(--neo-muted)]">
                {group.label}
              </div>
              {group.items.map((item) => {
                const active = isNavActive(pathname, item);
                return (
                  <NavAnchor
                    key={item.href}
                    href={item.href}
                    onNavigate={() => setMenuOpen(false)}
                    className={cn(
                      "mb-0.5 block w-full rounded-lg px-3 py-2 text-left text-sm shadow-none",
                      active
                        ? "bg-[var(--neo-surface-muted)] font-semibold text-[var(--neo-ink)]"
                        : "text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)]",
                    )}
                  >
                    {item.label}
                  </NavAnchor>
                );
              })}
            </div>
          ))}
        </nav>
      ) : null}
    </header>
  );
}
