"use client";

import { Download, LoaderCircle, RefreshCw, ChevronLeft, ChevronRight } from "lucide-react";
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
import { pickDisplayQuotaWindow } from "@/lib/grok-quota";

const PAGE_SIZE_OPTIONS = [20, 50, 100, 200] as const;

type PoolFilter = {
  provider: string;
  enabled: string;
  authStatus: string;
};

export default function GrokAccountsPage() {
  const [token, setToken] = useState<string | null>(() =>
    GROK_ADMIN_VIA_TNEXUS ? GROK_ADMIN_PROXY_TOKEN : getGrokAdminToken(),
  );
  const [items, setItems] = useState<GrokAccountView[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState<number>(50);
  const [filter, setFilter] = useState<PoolFilter>({ provider: "", enabled: "", authStatus: "" });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [reloadKey, setReloadKey] = useState(0);
  const [quotaByAccount, setQuotaByAccount] = useState<Record<number, GrokQuotaWindow | null>>({});
  const [quotaErrorByAccount, setQuotaErrorByAccount] = useState<Record<number, string>>({});

  const [editTarget, setEditTarget] = useState<GrokAccountView | null>(null);
  const [detailTarget, setDetailTarget] = useState<GrokAccountView | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [bulkBusy, setBulkBusy] = useState(false);

  const pageCount = Math.max(1, Math.ceil(total / pageSize));

  const refreshPageQuota = useCallback(async () => {
    if (!token || items.length === 0) return;
    const settled = await Promise.allSettled(
      items.map((a) => grokAdminApi.getQuotaWindows(token, a.id)),
    );
    setQuotaByAccount((prev) => {
      const next: Record<number, GrokQuotaWindow | null> = { ...prev };
      items.forEach((a, i) => {
        if (settled[i].status === "fulfilled") {
          next[a.id] = pickDisplayQuotaWindow((settled[i] as PromiseFulfilledResult<GrokQuotaWindow[]>).value);
        }
      });
      return next;
    });
    setQuotaErrorByAccount((prev) => {
      const next: Record<number, string> = { ...prev };
      items.forEach((a, i) => {
        if (settled[i].status === "rejected") {
          const reason = (settled[i] as PromiseRejectedResult).reason;
          next[a.id] = reason instanceof Error ? reason.message : String(reason);
        } else {
          delete next[a.id];
        }
      });
      return next;
    });
  }, [token, items]);

  const load = useCallback(
    async (pageNum: number, currentToken: string, size: number, f: PoolFilter) => {
      setLoading(true);
      setError("");
      try {
        const data: GrokAccountPage = await grokAdminApi.listAccounts(currentToken, {
          page: pageNum,
          pageSize: size,
          provider: f.provider || undefined,
          enabled: f.enabled || undefined,
          authStatus: f.authStatus || undefined,
        });
        const rows = data.items ?? [];
        setItems(rows);
        setTotal(data.total ?? 0);
        setPage(data.page ?? pageNum);
        if (data.page_size && data.page_size !== size) {
          setPageSize(data.page_size);
        }

        const settled = await Promise.allSettled(
          rows.map((a) => grokAdminApi.getQuotaWindows(currentToken, a.id)),
        );
        setQuotaByAccount((prev) => {
          const next: Record<number, GrokQuotaWindow | null> = { ...prev };
          rows.forEach((a, i) => {
            if (settled[i].status === "fulfilled") {
              next[a.id] = pickDisplayQuotaWindow((settled[i] as PromiseFulfilledResult<GrokQuotaWindow[]>).value);
            }
          });
          return next;
        });
        setQuotaErrorByAccount((prev) => {
          const next: Record<number, string> = { ...prev };
          rows.forEach((a, i) => {
            if (settled[i].status === "rejected") {
              const reason = (settled[i] as PromiseRejectedResult).reason;
              next[a.id] = reason instanceof Error ? reason.message : String(reason);
            } else {
              delete next[a.id];
            }
          });
          return next;
        });
      } catch (err) {
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
    if (token) void load(page, token, pageSize, filter);
    setReloadKey((k) => k + 1);
  }, [token, page, pageSize, filter, load]);

  const refreshAllQuotas = useCallback(async () => {
    if (!token) return;
    setBulkBusy(true);
    setError("");
    try {
      const result = await grokAdminApi.refreshAllQuotas(token, 128);
      await refreshPageQuota();
      setError(`批量额度刷新完成：成功 ${result.ok}，失败 ${result.fail}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBulkBusy(false);
    }
  }, [token, refreshPageQuota]);

  const nurtureCurrentPage = useCallback(async () => {
    if (!token || items.length === 0) return;
    setBulkBusy(true);
    setError("");
    try {
      const ids = items.filter((a) => a.enabled).map((a) => a.id);
      const result = await grokAdminApi.nurtureEnqueue(token, ids);
      setError(`已入队养号 ${result.enqueued} 个账号（队列深度 ${result.queue_depth}）`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBulkBusy(false);
    }
  }, [token, items]);

  useEffect(() => {
    if (!token) return;
    const timer = setTimeout(() => void load(1, token, pageSize, filter), 0);
    return () => clearTimeout(timer);
  }, [token, pageSize, filter.provider, filter.enabled, filter.authStatus, load]);

  useEffect(() => {
    if (!token || items.length === 0) return;
    const timer = setInterval(() => void refreshPageQuota(), 60_000);
    return () => clearInterval(timer);
  }, [token, items, refreshPageQuota]);

  const goPage = (next: number) => {
    if (!token || next < 1 || next > pageCount) return;
    void load(next, token, pageSize, filter);
  };

  return (
    <PageShell
      title="Grok 管理"
      subtitle="号池管理：分页 / 筛选 / 额度 / 导入 / 统计"
      badge="Phase 1"
      actions={
        token ? (
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => setImportOpen(true)} disabled={loading}>
              <Download className="size-4" />
              导入
            </Button>
            <Button variant="outline" size="sm" onClick={() => void refreshAllQuotas()} disabled={loading || bulkBusy}>
              同步全部额度
            </Button>
            <Button variant="outline" size="sm" onClick={() => void nurtureCurrentPage()} disabled={loading || bulkBusy || items.length === 0}>
              本页养号入队
            </Button>
            <Button variant="outline" size="sm" onClick={() => { handleReload(); void refreshPageQuota(); }} disabled={loading || bulkBusy}>
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
            void load(1, value, pageSize, filter);
          }}
        />
      ) : (
        <div className="flex flex-col gap-3">
          <GrokSummaryCards token={token} onError={setError} reloadKey={reloadKey} />

          <div className="neo-card flex flex-wrap items-end gap-3 p-3 text-sm">
            <label className="flex flex-col gap-1 text-xs text-[var(--neo-muted)]">
              Provider
              <select
                className="neo-input h-8 min-w-[120px] rounded-lg px-2"
                value={filter.provider}
                onChange={(e) => {
                  setFilter((f) => ({ ...f, provider: e.target.value }));
                  setPage(1);
                }}
              >
                <option value="">全部</option>
                <option value="grok_web">grok_web</option>
                <option value="grok_console">grok_console</option>
                <option value="grok_build">grok_build</option>
              </select>
            </label>
            <label className="flex flex-col gap-1 text-xs text-[var(--neo-muted)]">
              启用
              <select
                className="neo-input h-8 min-w-[100px] rounded-lg px-2"
                value={filter.enabled}
                onChange={(e) => {
                  setFilter((f) => ({ ...f, enabled: e.target.value }));
                  setPage(1);
                }}
              >
                <option value="">全部</option>
                <option value="true">启用</option>
                <option value="false">禁用</option>
              </select>
            </label>
            <label className="flex flex-col gap-1 text-xs text-[var(--neo-muted)]">
              认证状态
              <select
                className="neo-input h-8 min-w-[140px] rounded-lg px-2"
                value={filter.authStatus}
                onChange={(e) => {
                  setFilter((f) => ({ ...f, authStatus: e.target.value }));
                  setPage(1);
                }}
              >
                <option value="">全部</option>
                <option value="active">active</option>
                <option value="reauthRequired">reauthRequired</option>
                <option value="restricted">restricted</option>
                <option value="banned">banned</option>
              </select>
            </label>
            <label className="flex flex-col gap-1 text-xs text-[var(--neo-muted)]">
              每页
              <select
                className="neo-input h-8 min-w-[100px] rounded-lg px-2"
                value={pageSize}
                onChange={(e) => {
                  setPageSize(Number(e.target.value));
                  setPage(1);
                }}
              >
                {PAGE_SIZE_OPTIONS.map((n) => (
                  <option key={n} value={n}>
                    {n} 条
                  </option>
                ))}
              </select>
            </label>
            <div className="ml-auto text-xs text-[var(--neo-muted)]">
              共 {total} 个账号 · 第 {page}/{pageCount} 页
            </div>
          </div>

          {error ? <p className="text-sm text-rose-600">{error}</p> : null}
          {loading && items.length === 0 ? (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
              <LoaderCircle className="size-4 animate-spin" /> 加载中…
            </div>
          ) : (
            <>
              {Object.keys(quotaErrorByAccount).length > 0 && (
                <p
                  className="text-xs text-[var(--neo-muted)]"
                  title={Object.entries(quotaErrorByAccount)
                    .map(([id, msg]) => `#${id}: ${msg}`)
                    .join("\n")}
                >
                  {Object.keys(quotaErrorByAccount).length} 个账号额度读取失败
                </p>
              )}
              <GrokAccountsTable
                items={items}
                quotaByAccount={quotaByAccount}
                quotaErrorByAccount={quotaErrorByAccount}
                onEdit={(account) => setEditTarget(account)}
                onDetail={(account) => setDetailTarget(account)}
              />
            </>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs text-[var(--neo-muted)]">
              当前页 {items.length} 条 · 额度每 60s 自动刷新
            </span>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" disabled={page <= 1 || loading} onClick={() => goPage(page - 1)}>
                <ChevronLeft className="size-4" />
                上一页
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= pageCount || loading}
                onClick={() => goPage(page + 1)}
              >
                下一页
                <ChevronRight className="size-4" />
              </Button>
            </div>
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
