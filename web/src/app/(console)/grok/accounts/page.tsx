"use client";

import { Download, LoaderCircle, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { PageShell } from "@/components/admin/page-shell";
import { GrokAccountDetailDialog } from "@/components/grok/grok-account-detail-dialog";
import { GrokAccountEditDialog } from "@/components/grok/grok-account-edit-dialog";
import { GrokAccountsTable } from "@/components/grok/grok-accounts-table";
import { GrokActivityPanels } from "@/components/grok/grok-activity-panels";
import { GrokAccountHeatmap } from "@/components/grok/grok-heatmap";
import { GrokImportDialog } from "@/components/grok/grok-import-dialog";
import { GrokSummaryCards } from "@/components/grok/grok-summary-cards";
import { GrokTokenGateBody } from "@/components/grok/grok-token-gate";
import { Button } from "@/components/ui/button";
import {
  GROK_ADMIN_PROXY_TOKEN,
  GROK_ADMIN_VIA_TNEXUS,
  clearGrokAdminToken,
  getGrokAdminToken,
  GrokAdminAuthError,
  grokAdminApi,
  setGrokAdminToken,
  type GrokAccountPage,
  type GrokAccountView,
  type GrokQuotaWindow,
} from "@/lib/grok-admin";

const PAGE_SIZE = 50;
/** 额度列并发拉取的账号上限（列表接口不带额度；后端批量额度端点 TODO）。 */
const QUOTA_FETCH_LIMIT = 20;

export default function GrokAccountsPage() {
  const [token, setToken] = useState<string | null>(() =>
    GROK_ADMIN_VIA_TNEXUS ? GROK_ADMIN_PROXY_TOKEN : getGrokAdminToken(),
  );
  const [items, setItems] = useState<GrokAccountView[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  // 自增触发统计卡片/活跃面板/热力图重新拉取（「刷新」按钮）。
  const [reloadKey, setReloadKey] = useState(0);
  // 额度列：账号 → 额度窗口（当前页前 N 个并发拉取，容错；缺失显示「未知」）
  const [quotaByAccount, setQuotaByAccount] = useState<Record<number, GrokQuotaWindow | null>>({});

  // 对话框状态
  const [editTarget, setEditTarget] = useState<GrokAccountView | null>(null);
  const [detailTarget, setDetailTarget] = useState<GrokAccountView | null>(null);
  const [importOpen, setImportOpen] = useState(false);

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
        // 额度列：仅对当前页前 N 个账号并发拉取（列表接口不带额度窗口）
        const targets = (data.items ?? []).slice(0, QUOTA_FETCH_LIMIT);
        const settled = await Promise.allSettled(
          targets.map((a) => grokAdminApi.getQuotaWindows(currentToken, a.id)),
        );
        setQuotaByAccount((prev) => {
          const next: Record<number, GrokQuotaWindow | null> = { ...prev };
          targets.forEach((a, i) => {
            next[a.id] = settled[i].status === "fulfilled" ? (settled[i].value[0] ?? null) : null;
          });
          return next;
        });
      } catch (err) {
        // 401：会话失效 → 清 token 回登录门禁。
        if (err instanceof GrokAdminAuthError) {
          clearGrokAdminToken();
          setToken(null);
          setItems([]);
          setTotal(0);
          setError("管理员会话已过期，请重新登录");
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
        setItems([]);
        setTotal(0);
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  const handleReload = useCallback(() => {
    if (token) void load(page, token);
    setReloadKey((k) => k + 1);
  }, [token, page, load]);

  useEffect(() => {
    if (!token) return;
    // setTimeout 0：避免 effect 内同步 setState（react-compiler 规则）。
    const timer = setTimeout(() => void load(1, token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  return (
    <PageShell
      title="Grok 管理"
      subtitle="grok-admin 账号管理（列表 / 编辑 / 导入 / 统计 / 详情）"
      badge="Phase 1"
      actions={
        token ? (
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setImportOpen(true)}
              disabled={loading}
            >
              <Download className="size-4" />
              导入
            </Button>
            <Button variant="outline" size="sm" onClick={handleReload} disabled={loading}>
              {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
              刷新
            </Button>
          </div>
        ) : undefined
      }
    >
      {!token ? (
        <GrokTokenGateBody
          onToken={(value) => {
            setGrokAdminToken(value);
            setToken(value);
            void load(1, value);
          }}
        />
      ) : (
        <div className="flex flex-col gap-3">
          <GrokSummaryCards token={token} onError={setError} reloadKey={reloadKey} />
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
            <span>
              共 {total} 个账号 · 第 {page} 页（每页 {PAGE_SIZE}）
            </span>
            <button
              type="button"
              className="text-[var(--neo-muted)] underline-offset-2 hover:underline"
              onClick={() => {
                clearGrokAdminToken();
                setToken(null);
                setItems([]);
                setTotal(0);
                setError("");
              }}
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
            <GrokAccountsTable
              items={items}
              quotaByAccount={quotaByAccount}
              onEdit={(account) => setEditTarget(account)}
              onDetail={(account) => setDetailTarget(account)}
            />
          )}
          <div className="mt-2 text-[10px] text-[var(--neo-muted)]">
            额度列仅对当前页前 {QUOTA_FETCH_LIMIT} 个账号拉取（列表接口不带额度；后端批量额度端点 TODO）
          </div>
          <GrokActivityPanels token={token} reloadKey={reloadKey} />
          <GrokAccountHeatmap token={token} reloadKey={reloadKey} />
        </div>
      )}

      {token ? (
        <>
          <GrokAccountEditDialog
            open={editTarget !== null}
            account={editTarget}
            token={token}
            onOpenChange={(open) => {
              if (!open) setEditTarget(null);
            }}
            onSaved={handleReload}
          />
          <GrokAccountDetailDialog
            open={detailTarget !== null}
            account={detailTarget}
            token={token}
            onOpenChange={(open) => {
              if (!open) setDetailTarget(null);
            }}
            onEdit={(account) => {
              setDetailTarget(null);
              setEditTarget(account);
            }}
          />
          <GrokImportDialog
            open={importOpen}
            onOpenChange={setImportOpen}
            token={token}
            onImported={handleReload}
          />
        </>
      ) : null}
    </PageShell>
  );
}
