"use client";

import { KeyRound } from "lucide-react";
import { useCallback, useState } from "react";
import { ElevatedCard } from "@/components/admin/page-shell";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  clearGrokAdminToken,
  getGrokAdminToken,
  setGrokAdminToken,
} from "@/lib/grok-admin";

/**
 * grok-admin 访问令牌门：无 token 时渲染说明卡（粘贴 Bearer JWT 存 localStorage），
 * 有 token 时调用 `children(token)`。grok-admin 使用独立 Bearer JWT（HS256），
 * 与 TNexus 会话登录是两套体系（G6 统一登录前）。
 */
export function GrokTokenGate({
  children,
}: {
  children: (token: string) => React.ReactNode;
}) {
  const [token, setToken] = useState<string | null>(() => getGrokAdminToken());
  const [tokenInput, setTokenInput] = useState("");
  const [error, setError] = useState("");

  const saveToken = useCallback(() => {
    const value = tokenInput.trim();
    if (!value) return;
    setGrokAdminToken(value);
    setToken(value);
    setTokenInput("");
    setError("");
  }, [tokenInput]);

  const clearToken = useCallback(() => {
    clearGrokAdminToken();
    setToken(null);
    setError("");
  }, []);

  if (!token) {
    return (
      <ElevatedCard className="flex max-w-xl flex-col gap-3 p-6">
        <div className="flex items-center gap-2 text-sm font-medium text-[var(--neo-ink)]">
          <KeyRound className="size-4 text-[var(--neo-muted)]" />
          需要 grok-admin 访问令牌
        </div>
        <p className="text-sm leading-relaxed text-[var(--neo-muted)]">
          grok-admin 使用独立的 Bearer JWT（HS256），与 TNexus 会话登录是两套体系。
          粘贴管理员 access token 后，页面会保存到本地（localStorage）并加载数据。
        </p>
        <p className="text-xs text-[var(--neo-muted)] opacity-70">
          TODO（G6）：统一登录体系后改为会话自动换取 token，移除手动粘贴。
        </p>
        <div className="flex gap-2">
          <Input
            type="password"
            placeholder="粘贴 grok-admin Bearer token"
            value={tokenInput}
            onChange={(e) => setTokenInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") saveToken();
            }}
            className="flex-1"
          />
          <Button size="sm" onClick={saveToken} disabled={!tokenInput.trim()}>
            保存并加载
          </Button>
        </div>
        {error ? <p className="text-sm text-rose-600">{error}</p> : null}
      </ElevatedCard>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex justify-end text-xs text-[var(--neo-muted)]">
        <button
          type="button"
          className="underline-offset-2 hover:underline"
          onClick={clearToken}
        >
          清除令牌
        </button>
      </div>
      {children(token)}
    </div>
  );
}
