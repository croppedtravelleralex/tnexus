"use client";

import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { useAuth } from "@/lib/auth";

export function SiteHeader({ variant = "default" }: { variant?: "default" | "home" }) {
  const { user, loading, logout } = useAuth();
  const router = useRouter();

  const isHome = variant === "home";

  return (
    <header
      className={
        isHome
          ? "flex h-14 shrink-0 items-center justify-between border-b border-violet-200/50 bg-white/65 px-6 backdrop-blur-md"
          : "flex h-14 shrink-0 items-center justify-between border-b border-zinc-200 bg-white px-6"
      }
    >
      <Link
        href="/"
        className={`flex items-center gap-2 text-base font-semibold tracking-tight ${isHome ? "text-[#4b4469]" : "text-zinc-900"}`}
      >
        <Image src="/logo.png" alt="TNexus" width={32} height={32} className="rounded-lg" />
        TNexus
      </Link>
      <nav className="flex items-center gap-1">
        <Link href="/studio">
          <Button variant="ghost" size="sm">
            工作台
          </Button>
        </Link>
        <Link href="/history">
          <Button variant="ghost" size="sm">
            历史
          </Button>
        </Link>
        <Link href="/settings">
          <Button variant="ghost" size="sm">
            设置
          </Button>
        </Link>
        {!loading && user ? (
          <div className="ml-2 flex items-center gap-2 border-l border-zinc-200 pl-3">
            <span className="hidden text-sm text-zinc-600 sm:inline">
              {user.display_name || user.email}
            </span>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void logout().then(() => router.push("/login"))}
            >
              退出
            </Button>
          </div>
        ) : (
          <Link href="/login">
            <Button size="sm">登录</Button>
          </Link>
        )}
      </nav>
    </header>
  );
}
