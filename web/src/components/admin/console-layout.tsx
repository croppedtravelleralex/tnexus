"use client";

import { usePathname } from "next/navigation";
import { NavProgress } from "@/components/admin/nav-progress";
import { TopNav } from "@/components/admin/top-nav";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/auth";
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

  const isStudio = pathname === "/studio" || pathname.startsWith("/studio/") || pathname.startsWith("/history");

  return (
    <div className="console-shell flex min-h-screen flex-col">
      <NavProgress />
      <TopNav />
      <main className={isStudio ? "relative z-0 flex min-h-0 flex-1 flex-col overflow-hidden" : "relative z-0 flex min-h-0 flex-1 flex-col px-0"}>
        {children}
      </main>
    </div>
  );
}
