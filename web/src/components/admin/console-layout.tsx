"use client";

import { usePathname } from "next/navigation";
import { NavProgress } from "@/components/admin/nav-progress";
import { TopNav } from "@/components/admin/top-nav";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { isAdminRoute } from "@/lib/nav";
import { useEffect } from "react";
import { useRouter } from "next/navigation";

export function ConsoleLayout({ children }: { children: React.ReactNode }) {
  const { user, bootstrapping } = useAuth();
  const router = useRouter();
  const pathname = usePathname();

  useEffect(() => {
    if (!bootstrapping && !user && pathname !== "/login" && pathname !== "/login/") {
      router.replace("/login/");
    }
  }, [bootstrapping, user, router, pathname]);

  if (bootstrapping && !user) {
    return (
      <div className="console-shell flex min-h-screen items-center justify-center">
        <Loader2 className="size-8 animate-spin text-[var(--neo-muted)]" />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  // 客户端门控：非管理员访问管理员路由时展示友好提示。
  // 安全边界由服务端 API 强制执行，此处为纵深防御与 UX 改善。
  if (isAdminRoute(pathname) && user.role !== "admin") {
    return (
      <div className="console-shell flex min-h-screen flex-col">
        <NavProgress />
        <TopNav />
        <main className="flex flex-1 flex-col items-center justify-center gap-3 px-4">
          <span className="select-none text-5xl font-bold tabular-nums text-[var(--neo-border)]">
            403
          </span>
          <p className="text-base font-semibold text-[var(--neo-ink)]">需要管理员权限</p>
          <p className="text-sm text-[var(--neo-muted)]">当前账户无权访问此页面</p>
          <a
            href="/studio/"
            className="mt-2 rounded-md bg-[var(--neo-primary-deep)] px-4 py-1.5 text-sm font-medium text-white hover:opacity-90"
          >
            返回工作台
          </a>
        </main>
      </div>
    );
  }

  const isStudio =
    pathname === "/studio" ||
    pathname.startsWith("/studio/") ||
    pathname.startsWith("/history");

  return (
    <div className="console-shell flex min-h-screen flex-col">
      <NavProgress />
      <TopNav />
      <main
        className={
          isStudio
            ? "relative z-0 flex min-h-0 flex-1 flex-col overflow-hidden"
            : "relative z-0 flex min-h-0 flex-1 flex-col px-0"
        }
      >
        {children}
      </main>
    </div>
  );
}
