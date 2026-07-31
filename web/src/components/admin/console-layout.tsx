"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { Loader2 } from "lucide-react";
import { ConsolePageCache } from "@/components/admin/console-page-cache";
import { NavProgress } from "@/components/admin/nav-progress";
import { TopNav } from "@/components/admin/top-nav";
import { useAuth } from "@/lib/auth";

const PREFETCH_ROUTES = [
  "/studio",
  "/accounts",
  "/image-manager",
  "/logs",
  "/ops",
  "/chat",
  "/settings",
] as const;

export function ConsoleLayout({ children }: { children: React.ReactNode }) {
  const { user, bootstrapping } = useAuth();
  const router = useRouter();
  const pathname = usePathname();
  const isStudio = pathname === "/studio" || pathname.startsWith("/history");

  useEffect(() => {
    if (!bootstrapping && !user && pathname !== "/login") {
      router.replace("/login");
    }
  }, [bootstrapping, user, router, pathname]);

  useEffect(() => {
    for (const href of PREFETCH_ROUTES) {
      if (href !== pathname) router.prefetch(href);
    }
  }, [router, pathname]);

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

  return (
    <div className="console-shell flex min-h-screen flex-col">
      <NavProgress />
      <TopNav />
      <main className={isStudio ? "flex min-h-0 flex-1 flex-col" : "flex min-h-0 flex-1 flex-col px-0"}>
        <ConsolePageCache>{children}</ConsolePageCache>
      </main>
    </div>
  );
}
