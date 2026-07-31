"use client";

import { useEffect, useState } from "react";
import { usePathname } from "next/navigation";

/** 路由切换时顶部细进度条，减轻「点了没反应」的体感卡顿 */
export function NavProgress() {
  const pathname = usePathname();
  const [active, setActive] = useState(false);

  useEffect(() => {
    setActive(true);
    const timer = window.setTimeout(() => setActive(false), 320);
    return () => window.clearTimeout(timer);
  }, [pathname]);

  if (!active) return null;

  return (
    <div className="pointer-events-none fixed inset-x-0 top-0 z-[60] h-0.5 overflow-hidden bg-transparent">
      <div className="h-full w-1/3 animate-[nav-progress_0.35s_ease-out_forwards] bg-[var(--neo-primary)]" />
    </div>
  );
}
