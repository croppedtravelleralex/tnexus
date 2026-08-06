"use client";

import { KeyRound, LoaderCircle, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { GrokAccountsTable } from "@/components/grok/grok-accounts-table";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  clearGrokAdminToken,
  getGrokAdminToken,
  grokAdminApi,
  setGrokAdminToken,
  type GrokAccountPage,
  type GrokAccountView,
} from "@/lib/grok-admin";

const PAGE_SIZE = 50;

export default function GrokAccountsPage() {
  const [token, setToken] = useState<string | null>(() => getGrokAdminToken());
  const [tokenInput, setTokenInput] = useState("");
  const [items, setItems] = useState<GrokAccountView[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(
    async (pageNum: number, currentToken: string) => {
      setLoading(true);
      setError("");
      try {
        const data: GrokAccountPage = await grokAdminApi.listAccounts(currentToken, {
          page: pageNum,
          pageSize: PAGE_SIZE,
        });
        setItems(data.items ?? []);
        setTotal(data.total ?? 0);
        setPage(data.page ?? pageNum);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
        setItems([]);
        setTotal(0);
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  const saveToken = () => {
    const value = tokenInput.trim();
    if (!value) return;
    setGrokAdminToken(value);
    setToken(value);
    setTokenInput("");
    void load(1, value);
  };

  const clearToken = () => {
    clearGrokAdminToken();
    setToken(null);
    setItems([]);
    setTotal(0);
    setError("");
  };

  useEffect(() => {
    if (token) void load(1, token);
  }, [token, load]);

  return (
    <PageShell
      title="Grok 管理"
      subtitle="grok-admin 账号管理（G6-P1 Phase 1，只读）"
      badge="Phase 1"
      actions={
        token ? (
          <Button variant="outline" size="sm" onClick={() => void load(page, token)} disabled={loading}>
            {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            刷新
          </Button>
        ) : undefined
      }
    >
      {!token ? (
        <ElevatedCard className="flex max-w-xl flex-col gap-3 p-6">
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--neo-ink)]">
            <KeyRound className="size-4 text-[var(--neo-muted)]" />
            需要 grok-admin 访问令牌
          </div>
          <p className="text-sm leading-relaxed text-[var(--neo-muted)]">
            grok-admin 使用独立的 Bearer JWT（HS256），与 TNexus 会话登录是两套体系。
            粘贴管理员 access token 后，页面会保存到本地（localStorage）并只读加载账号列表。
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
      ) : (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
            <span>
              共 {total} 个账号 · 第 {page} 页（每页 {PAGE_SIZE}）
            </span>
            <button
              type="button"
              className="text-[var(--neo-muted)] underline-offset-2 hover:underline"
              onClick={clearToken}
            >
              清除令牌
            </button>
          </div>
          {error ? <p className="text-sm text-rose-600">{error}</p> : null}
          {loading && items.length === 0 ? (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
              <LoaderCircle className="size-4 animate-spin" /> 加载中…
            </div>
          ) : (
            <GrokAccountsTable items={items} />
          )}
        </div>
      )}
    </PageShell>
  );
}
