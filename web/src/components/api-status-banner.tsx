"use client";

import { useAuth } from "@/lib/auth";

export function ApiStatusBanner() {
  const { apiOnline, bootstrapping } = useAuth();
  if (bootstrapping || apiOnline) return null;

  return (
    <div className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-center text-sm text-amber-900">
      后端未连接：请在 WSL 中启动{" "}
      <code className="rounded bg-amber-100 px-1.5 py-0.5 text-xs">tnexus-api</code> 与{" "}
      <code className="rounded bg-amber-100 px-1.5 py-0.5 text-xs">tnexus-worker</code>
      （或运行 <code className="rounded bg-amber-100 px-1.5 py-0.5 text-xs">scripts/local-dev.sh</code>）
    </div>
  );
}
