"use client";

import { usePathname } from "next/navigation";
import { useRef, type ReactNode } from "react";

const MAX_CACHED_PAGES = 6;
const PINNED_PATHS = new Set(["/studio"]);

/** 重页不缓存，避免切换时主线程长时间阻塞导致「点击无反应」 */
function isHeavyRoute(pathname: string) {
  const p = pathname.replace(/\/$/, "") || "/";
  return p === "/accounts" || p === "/image-manager";
}

type CacheEntry = { node: ReactNode; lastAccess: number };

/**
 * 控制台页面保活：已访问过的路由保持挂载（display:none），切换时避免整页卸载重载。
 */
export function ConsolePageCache({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const cacheRef = useRef(new Map<string, CacheEntry>());

  if (isHeavyRoute(pathname)) {
    return <div className="flex min-h-0 flex-1 flex-col">{children}</div>;
  }

  const cache = cacheRef.current;
  const existing = cache.get(pathname);
  if (existing) {
    existing.lastAccess = Date.now();
    existing.node = children;
  } else {
    if (cache.size >= MAX_CACHED_PAGES) {
      let evictKey: string | null = null;
      let oldest = Infinity;
      for (const [key, entry] of cache.entries()) {
        if (PINNED_PATHS.has(key)) continue;
        if (entry.lastAccess < oldest) {
          oldest = entry.lastAccess;
          evictKey = key;
        }
      }
      if (evictKey) cache.delete(evictKey);
    }
    cache.set(pathname, { node: children, lastAccess: Date.now() });
  }

  const entries = [...cache.entries()];
  return (
    <>
      {entries.map(([path, entry]) => (
        <div
          key={path}
          className="flex min-h-0 flex-1 flex-col"
          style={{ display: path === pathname ? "flex" : "none" }}
          aria-hidden={path !== pathname}
          data-console-page={path}
        >
          {entry.node}
        </div>
      ))}
    </>
  );
}
