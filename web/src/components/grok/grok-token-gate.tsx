"use client";

import { KeyRound, LogIn } from "lucide-react";
import { useCallback, useState } from "react";
import { ElevatedCard } from "@/components/admin/page-shell";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import {
  clearGrokAdminToken,
  getGrokAdminToken,
  grokAdminApi,
  setGrokAdminToken,
} from "@/lib/grok-admin";
/**
 * grok-admin 访问令牌门：无 token 时渲染 [`GrokTokenGateBody`]（粘贴 Bearer JWT 或
 * 用户名/密码登录），有 token 时调用 `children(token)`。grok-admin 使用独立 Bearer
 * JWT（HS256），与 TNexus 会话登录是两套体系（G6 统一登录前）。
 */
export function GrokTokenGate({
  children,
}: {
  children: (token: string) => React.ReactNode;
}) {
  const [token, setToken] = useState<string | null>(() => getGrokAdminToken());
  const clearToken = useCallback(() => {
    clearGrokAdminToken();
    setToken(null);
  }, []);

  if (!token) {
    return (
      <GrokTokenGateBody
        onToken={(value) => {
          setGrokAdminToken(value);
          setToken(value);
        }}
      />
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

/**
 * 无令牌时的门禁体：粘贴 Bearer token 或账号登录（POST /admin/auth/login）。
 * 供 `GrokTokenGate` 与账号页内联门禁复用，登录成功后回传 token。
 */
export function GrokTokenGateBody({
  onToken,
}: {
  onToken: (token: string) => void;
}) {
  const [mode, setMode] = useState<"paste" | "login">("paste");
  const [tokenInput, setTokenInput] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const saveToken = useCallback(() => {
    const value = tokenInput.trim();
    if (!value) return;
    onToken(value);
  }, [tokenInput, onToken]);

  const doLogin = useCallback(async () => {
    const user = username.trim();
    if (!user || !password) {
      setError("请输入用户名和密码");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const res = await grokAdminApi.login(user, password);
      onToken(res.tokens.access_token);
    } catch (err) {
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setBusy(false);
    }
  }, [username, password, onToken]);

  return (
    <ElevatedCard className="flex max-w-xl flex-col gap-3 p-6">
      <div className="flex items-center gap-2 text-sm font-medium text-[var(--neo-ink)]">
        <KeyRound className="size-4 text-[var(--neo-muted)]" />
        需要 grok-admin 访问令牌
      </div>
      <p className="text-sm leading-relaxed text-[var(--neo-muted)]">
        grok-admin 使用独立的 Bearer JWT（HS256），与 TNexus 会话登录是两套体系。
        可登录管理员账号自动换取 token，或直接粘贴已有 access token。
      </p>
      <div className="flex gap-2 text-xs">
        <button
          type="button"
          className={`rounded-full px-3 py-1 ${mode === "login" ? "bg-pink-100 text-pink-700" : "text-[var(--neo-muted)] hover:bg-stone-100"}`}
          onClick={() => {
            setMode("login");
            setError("");
          }}
        >
          账号登录
        </button>
        <button
          type="button"
          className={`rounded-full px-3 py-1 ${mode === "paste" ? "bg-pink-100 text-pink-700" : "text-[var(--neo-muted)] hover:bg-stone-100"}`}
          onClick={() => {
            setMode("paste");
            setError("");
          }}
        >
          粘贴令牌
        </button>
      </div>
      {mode === "login" ? (
        <div className="flex flex-col gap-2">
          <div className="space-y-1">
            <Label htmlFor="grok-admin-username">用户名</Label>
            <Input
              id="grok-admin-username"
              autoComplete="username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="grok-admin-password">密码</Label>
            <Input
              id="grok-admin-password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void doLogin();
              }}
            />
          </div>
          <Button
            size="sm"
            onClick={() => void doLogin()}
            disabled={busy || !username.trim() || !password}
          >
            <LogIn className="size-4" />
            {busy ? "登录中…" : "登录"}
          </Button>
        </div>
      ) : (
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
      )}
      {error ? <p className="text-sm text-rose-600">{error}</p> : null}
    </ElevatedCard>
  );
}
