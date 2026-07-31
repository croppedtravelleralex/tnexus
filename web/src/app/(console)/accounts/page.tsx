"use client";

import { CloudUpload, Download, LoaderCircle, LogIn, Pause, Play, RefreshCw, Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { MockLineChart } from "@/components/admin/mock-chart";
import { AccountImportDialog } from "@/components/accounts/account-import-dialog";
import { AccountUsageHeatstrip } from "@/components/accounts/account-usage-heatstrip";
import {
  activityMatrixToWeights,
  BindingSgHeatmap,
  bindingMatrixPeak,
} from "@/components/accounts/BindingSgHeatmap";
import { CfStatusLight, cfDaysForAccount } from "@/components/accounts/CfStatusLight";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { accountsApi, type Account, type AccountListStats } from "@/lib/api";
import { fetchWithCache, invalidateCache } from "@/lib/api-cache";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 50;
const MAX_REFRESH = 50;
const USAGE_DAYS = 6;

function isManualSchedulingEnabled(account: Account) {
  const receive = String(account.panda_receive_state ?? "").trim().toLowerCase();
  if (!receive) return true;
  return receive === "verified_ready" || receive === "verified" || receive === "local_verified";
}

function proxyEndpoint(account: Account) {
  const egress = String(account.proxy_egress_ip ?? "").trim();
  if (egress) return egress;
  const raw = String(account.proxy ?? "").trim();
  if (!raw) return "默认出口";
  try {
    const parsed = new URL(raw);
    return parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname;
  } catch {
    return raw.replace(/^[a-z]+:\/\//i, "").split("/")[0] || "账号代理";
  }
}

function statCards(stats: AccountListStats | undefined) {
  const s = stats ?? {
    total: 0,
    active: 0,
    limited: 0,
    abnormal: 0,
    disabled: 0,
    total_quota: 0,
    schedulable: 0,
    available_image_quota: 0,
  };
  return [
    { label: "账户总数", value: s.total },
    { label: "正常账户", value: s.active },
    { label: "可调度", value: s.schedulable ?? s.scheduling_enabled ?? 0 },
    { label: "可用额度", value: s.available_image_quota ?? 0 },
    { label: "限流账户", value: s.limited },
    { label: "禁用账户", value: s.disabled },
    { label: "报错账户", value: s.abnormal },
    { label: "总额度", value: s.total_quota },
  ] as const;
}

function flowFromActivity(
  items: Array<{ date: string; registered: number; uploaded: number; received: number; deleted: number; images?: number; dialogues?: number }>,
) {
  const labels = items.map((i) => i.date.slice(5));
  return {
    labels,
    register: items.map((i) => i.registered + i.received + i.uploaded),
    image: items.map((i) => (i.images ?? 0) + (i.dialogues ?? 0)),
  };
}

async function pollProgress(
  fetcher: (id: string) => Promise<{ done?: boolean; error?: string | null }>,
  progressId: string,
) {
  for (let i = 0; i < 600; i += 1) {
    const p = await fetcher(progressId);
    if (p.done) {
      if (p.error) throw new Error(p.error);
      return;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error("刷新超时");
}

export default function AccountsPage() {
  const [items, setItems] = useState<Account[]>([]);
  const [stats, setStats] = useState<AccountListStats>();
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [flow, setFlow] = useState<ReturnType<typeof flowFromActivity> | null>(null);
  const [schedulingBusy, setSchedulingBusy] = useState<Set<string>>(new Set());
  const [bulkScheduling, setBulkScheduling] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [relogging, setRelogging] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [usageDates, setUsageDates] = useState<string[]>([]);
  const [usageByEmail, setUsageByEmail] = useState<Record<string, Array<{ date: string; images: number; dialogues: number }>>>({});
  const [schedBreakdown, setSchedBreakdown] = useState<Record<string, number>>({});
  const [bindingSlots, setBindingSlots] = useState<Record<string, Record<string, number[][]>>>({});

  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  const loadUsage = useCallback(async () => {
    try {
      const usage = await accountsApi.usageRecent(USAGE_DAYS);
      setUsageDates(usage.dates ?? []);
      setUsageByEmail(usage.by_email ?? {});
    } catch {
      // optional
    }
    try {
      const breakdown = await accountsApi.schedulableBreakdown();
      setSchedBreakdown(breakdown.buckets ?? {});
    } catch {
      // optional
    }
    try {
      const slots = await accountsApi.bindingSlots({ week_offset: 0 });
      setBindingSlots(slots.by_binding ?? {});
    } catch {
      // optional
    }
  }, []);

  const load = useCallback(
    async (options?: { force?: boolean; background?: boolean; page?: number }) => {
      const targetPage = options?.page ?? page;
      if (!options?.background) setLoading(true);
      setError("");
      try {
        const offset = (targetPage - 1) * PAGE_SIZE;
        const [listResult, activityResult] = await Promise.all([
          fetchWithCache(
            `accounts:list:${offset}:${PAGE_SIZE}`,
            () => accountsApi.list({ offset, limit: PAGE_SIZE }),
            45_000,
            { force: options?.force },
          ),
          fetchWithCache("accounts:activity14", () => accountsApi.activityDaily(14), 60_000, {
            force: options?.force,
          }),
        ]);
        const list = listResult.data;
        const activity = activityResult.data;
        setItems(list.items ?? []);
        setStats(list.stats);
        setTotal(list.total ?? list.items?.length ?? 0);
        setFlow(flowFromActivity(activity.items ?? []));
      } catch (err) {
        setError(err instanceof Error ? err.message : "加载号池失败");
      } finally {
        setLoading(false);
      }
      void loadUsage();
    },
    [loadUsage, page],
  );

  useEffect(() => {
    void load({ background: items.length > 0 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return items;
    return items.filter((row) => (row.email ?? "").toLowerCase().includes(q));
  }, [items, search]);

  const selectedOnPage = filtered.filter((r) => selected.has(r.access_token));
  const allPageSelected = filtered.length > 0 && filtered.every((r) => selected.has(r.access_token));

  const bindingGroups = useMemo(() => {
    const map = new Map<string, Account[]>();
    for (const row of items) {
      const key = proxyEndpoint(row);
      const list = map.get(key) ?? [];
      list.push(row);
      map.set(key, list);
    }
    return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [items]);

  const cards = statCards(stats);

  const toggleSelectAllPage = () => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (allPageSelected) {
        for (const row of filtered) next.delete(row.access_token);
      } else {
        for (const row of filtered) next.add(row.access_token);
      }
      return next;
    });
  };

  const onRefresh = () => {
    invalidateCache("accounts:");
    void load({ force: true, page });
  };

  const onReloadStorage = async () => {
    try {
      await accountsApi.reloadFromStorage();
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "热加载失败");
    }
  };

  const tokensForAction = () => {
    const fromSelected = Array.from(selected);
    if (fromSelected.length > 0) return fromSelected;
    return filtered.map((r) => r.access_token).filter(Boolean);
  };

  const onRefreshAccounts = async () => {
    const tokens = tokensForAction();
    if (tokens.length === 0) return;
    if (tokens.length > MAX_REFRESH) {
      setError(`单次最多刷新 ${MAX_REFRESH} 个账号`);
      return;
    }
    setRefreshing(true);
    setError("");
    try {
      const { progress_id } = await accountsApi.refresh(tokens);
      await pollProgress(accountsApi.refreshProgress, progress_id);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "批量刷新失败");
    } finally {
      setRefreshing(false);
    }
  };

  const onReloginAccounts = async () => {
    const tokens =
      selected.size > 0
        ? Array.from(selected)
        : filtered.filter((r) => r.status === "异常").map((r) => r.access_token);
    if (tokens.length === 0) {
      setError("请选择账号或确保当前页有异常账号");
      return;
    }
    setRelogging(true);
    setError("");
    try {
      const { progress_id } = await accountsApi.reLogin(tokens);
      await pollProgress(accountsApi.reLoginProgress, progress_id);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "批量重登失败");
    } finally {
      setRelogging(false);
    }
  };

  const onBulkScheduling = async (enabled: boolean) => {
    const tokens = tokensForAction();
    if (tokens.length === 0) return;
    setBulkScheduling(true);
    try {
      await accountsApi.schedulingBulk(tokens, enabled);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "调度更新失败");
    } finally {
      setBulkScheduling(false);
    }
  };

  const onToggleScheduling = async (account: Account) => {
    if (!account.access_token) return;
    const next = !isManualSchedulingEnabled(account);
    setSchedulingBusy((prev) => new Set(prev).add(account.access_token));
    try {
      await accountsApi.setScheduling(account.access_token, next);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "调度切换失败");
    } finally {
      setSchedulingBusy((prev) => {
        const copy = new Set(prev);
        copy.delete(account.access_token);
        return copy;
      });
    }
  };

  const onExport = async () => {
    const tokens = tokensForAction();
    if (tokens.length === 0) {
      setError("当前没有可导出的账号");
      return;
    }
    setExporting(true);
    setError("");
    try {
      await accountsApi.exportJson(tokens);
    } catch (err) {
      setError(err instanceof Error ? err.message : "导出失败");
    } finally {
      setExporting(false);
    }
  };

  return (
    <PageShell
      title="号池管理"
      actions={
        <>
          <AccountImportDialog disabled={loading} onImported={() => void load({ force: true, page })} />
          <Button size="sm" variant="toolbar" className="h-8 gap-1.5" onClick={onRefresh}>
            <RefreshCw className="size-3.5" /> 刷新列表
          </Button>
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 gap-1.5"
            disabled={refreshing}
            onClick={() => void onRefreshAccounts()}
          >
            {refreshing ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            刷新账号
          </Button>
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 gap-1.5"
            disabled={relogging}
            onClick={() => void onReloginAccounts()}
          >
            {relogging ? <LoaderCircle className="size-3.5 animate-spin" /> : <LogIn className="size-3.5" />}
            重登
          </Button>
          <Button size="sm" className="h-8 gap-1.5" disabled={bulkScheduling} onClick={() => void onBulkScheduling(true)}>
            <Play className="size-3.5" /> 进调度
          </Button>
          <Button size="sm" variant="toolbar" className="h-8 gap-1.5" disabled={bulkScheduling} onClick={() => void onBulkScheduling(false)}>
            <Pause className="size-3.5" /> 出调度
          </Button>
          <Button size="sm" variant="toolbar" className="h-8 gap-1.5" onClick={() => void onReloadStorage()}>
            <CloudUpload className="size-3.5" /> 热加载
          </Button>
          <Button size="sm" variant="toolbar" className="h-8 gap-1.5" disabled={exporting} onClick={() => void onExport()}>
            <Download className="size-3.5" /> 导出
          </Button>
        </>
      }
    >
      {error ? (
        <ElevatedCard className="mb-4 border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</ElevatedCard>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {cards.map((c) => (
          <ElevatedCard key={c.label} className="p-4">
            <p className="text-xs font-medium text-[var(--neo-muted)]">{c.label}</p>
            <p className="mt-2 text-2xl font-semibold text-[var(--neo-ink)]">{c.value}</p>
          </ElevatedCard>
        ))}
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <ElevatedCard className="p-4">
          <p className="mb-3 text-sm font-medium text-[var(--neo-ink)]">账号流水 — 注册/入库</p>
          {flow ? <MockLineChart labels={flow.labels} series={flow.register} /> : <p className="text-sm text-[var(--neo-muted)]">暂无数据</p>}
        </ElevatedCard>
        <ElevatedCard className="p-4">
          <p className="mb-3 text-sm font-medium text-[var(--neo-ink)]">账号流水 — 生图/对话</p>
          {flow ? <MockLineChart labels={flow.labels} series={flow.image} /> : <p className="text-sm text-[var(--neo-muted)]">暂无数据</p>}
        </ElevatedCard>
      </div>

      {Object.keys(bindingSlots).length > 0 ? (
        <ElevatedCard className="mt-4 p-4">
          <p className="mb-3 text-sm font-medium text-[var(--neo-ink)]">按 IP 绑定组活动热力图</p>
          <div className="flex flex-wrap gap-6">
            {Object.entries(bindingSlots).map(([binding, metrics]) => {
              const matrix = metrics.images_api ?? metrics.images_chat;
              const peak = bindingMatrixPeak(matrix);
              return (
                <BindingSgHeatmap
                  key={binding}
                  label={`${binding} · 峰值 ${peak}`}
                  weights={activityMatrixToWeights(matrix)}
                />
              );
            })}
          </div>
        </ElevatedCard>
      ) : null}

      {bindingGroups.length > 0 ? (
        <ElevatedCard className="mt-4 p-4">
          <p className="mb-2 text-sm font-medium text-[var(--neo-ink)]">当前页按出口分组</p>
          <div className="flex flex-wrap gap-2">
            {bindingGroups.map(([endpoint, rows]) => (
              <Badge key={endpoint} variant="muted">
                {endpoint}: {rows.length}
              </Badge>
            ))}
          </div>
        </ElevatedCard>
      ) : null}

      {Object.keys(schedBreakdown).length > 0 ? (
        <ElevatedCard className="mt-4 p-4">
          <p className="mb-2 text-sm font-medium text-[var(--neo-ink)]">可调度分布</p>
          <div className="flex flex-wrap gap-2">
            {Object.entries(schedBreakdown).map(([bucket, count]) => (
              <Badge key={bucket} variant="muted">
                {bucket}: {count}
              </Badge>
            ))}
          </div>
        </ElevatedCard>
      ) : null}

      <ElevatedCard className="mt-4 overflow-hidden">
        <div className="flex flex-wrap items-center gap-2 border-b border-[var(--neo-border)] px-4 py-3">
          <div className="relative min-w-[200px] flex-1">
            <Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-[var(--neo-muted)]" />
            <Input
              placeholder="搜索邮箱（当前页）"
              className="h-8 pl-8 text-sm"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
          <Badge variant="muted">
            已选 {selected.size} · 本页 {filtered.length} / 总 {total}
          </Badge>
          <div className="flex items-center gap-1">
            <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage((p) => Math.max(1, p - 1))}>
              上一页
            </Button>
            <span className="text-xs text-[var(--neo-muted)]">
              {page} / {pageCount}
            </span>
            <Button size="sm" variant="outline" disabled={page >= pageCount} onClick={() => setPage((p) => p + 1)}>
              下一页
            </Button>
          </div>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[960px] text-left text-sm">
            <thead className="neo-table-head">
              <tr>
                <th className="px-3 py-2.5">
                  <input type="checkbox" checked={allPageSelected} onChange={toggleSelectAllPage} aria-label="全选本页" />
                </th>
                <th className="px-4 py-2.5 font-medium">邮箱</th>
                <th className="px-4 py-2.5 font-medium">状态</th>
                <th className="px-4 py-2.5 font-medium">CF</th>
                <th className="px-4 py-2.5 font-medium">调度</th>
                <th className="px-4 py-2.5 font-medium">记录</th>
                <th className="px-4 py-2.5 font-medium">出口</th>
                <th className="px-4 py-2.5 font-medium">额度</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((row) => (
                <tr key={row.email ?? row.access_token} className="border-t border-[var(--neo-border)] neo-row-hover">
                  <td className="px-3 py-3">
                    <input
                      type="checkbox"
                      checked={selected.has(row.access_token)}
                      onChange={() =>
                        setSelected((prev) => {
                          const next = new Set(prev);
                          if (next.has(row.access_token)) next.delete(row.access_token);
                          else next.add(row.access_token);
                          return next;
                        })
                      }
                      aria-label={`选择 ${row.email ?? "账号"}`}
                    />
                  </td>
                  <td className="px-4 py-3 font-medium text-[var(--neo-ink)]">{row.email ?? "—"}</td>
                  <td className="px-4 py-3">
                    <Badge variant={row.status === "正常" ? "success" : "warning"}>{row.status}</Badge>
                  </td>
                  <td className="px-4 py-3">
                    <CfStatusLight days={cfDaysForAccount(row)} />
                  </td>
                  <td className="px-4 py-3">
                    {(() => {
                      const inSchedule = isManualSchedulingEnabled(row);
                      const busy = schedulingBusy.has(row.access_token) || bulkScheduling;
                      return (
                        <button
                          type="button"
                          className={cn(
                            "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium transition",
                            inSchedule ? "bg-emerald-50 text-emerald-700" : "bg-amber-50 text-amber-700",
                          )}
                          onClick={() => void onToggleScheduling(row)}
                          disabled={busy}
                        >
                          {busy ? <LoaderCircle className="size-3.5 animate-spin" /> : inSchedule ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
                          {inSchedule ? "调度中" : "已隔离"}
                        </button>
                      );
                    })()}
                  </td>
                  <td className="px-4 py-3">
                    <AccountUsageHeatstrip
                      days={
                        usageByEmail[String(row.email ?? "").trim().toLowerCase()] ??
                        usageDates.map((date) => ({ date, images: 0, dialogues: 0 }))
                      }
                    />
                  </td>
                  <td className="px-4 py-3 text-xs text-[var(--neo-muted)]">{proxyEndpoint(row)}</td>
                  <td className="px-4 py-3 font-medium">{row.quota}</td>
                </tr>
              ))}
              {!loading && filtered.length === 0 ? (
                <tr>
                  <td colSpan={8} className="px-4 py-8 text-center text-[var(--neo-muted)]">
                    暂无账户
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      </ElevatedCard>
    </PageShell>
  );
}
