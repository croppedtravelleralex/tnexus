"use client";

import { CloudUpload, Download, LoaderCircle, LogIn, Pause, Play, RefreshCw, Search, Sun, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AccountEditDialog } from "@/components/accounts/account-edit-dialog";
import { AccountImportDialog } from "@/components/accounts/account-import-dialog";
import { AccountsActivityPanels } from "@/components/accounts/accounts-activity-panels";
import { AccountsDataTable, type AccountViewMode, type SortKey } from "@/components/accounts/accounts-data-table";
import type { HeatmapTimezone } from "@/components/accounts/BindingActivityHeatmapToolbar";
import { NurtureWeightDialog } from "@/components/accounts/nurture-weight-dialog";
import { OutlookRecoveryPanel } from "@/components/accounts/outlook-recovery-panel";
import { RefreshAllPanel } from "@/components/accounts/refresh-all-panel";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  accountsApi,
  opsApi,
  type Account,
  type AccountListStats,
  type BindingSlotsResponse,
  type IpNurtureBinding,
  type IpNurturePreset,
} from "@/lib/api";
import { displayAccountType } from "@/lib/account-display";
import { sortAccounts } from "@/lib/account-sort";
import { fetchWithCache, invalidateCache } from "@/lib/api-cache";

const PAGE_SIZE = 50;
const MAX_REFRESH = 50;
const MAX_PRIME = 50;
const USAGE_DAYS = 6;

function isManualSchedulingEnabled(account: Account) {
  const receive = String(account.panda_receive_state ?? "").trim().toLowerCase();
  if (!receive) return true;
  return receive === "verified_ready" || receive === "verified" || receive === "local_verified";
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
  const [activityRefreshToken, setActivityRefreshToken] = useState(0);
  const [schedulingBusy, setSchedulingBusy] = useState<Set<string>>(new Set());
  const [bulkScheduling, setBulkScheduling] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [relogging, setRelogging] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [usageDates, setUsageDates] = useState<string[]>([]);
  const [usageByEmail, setUsageByEmail] = useState<Record<string, Array<{ date: string; images: number; dialogues: number }>>>({});
  const [schedBreakdown, setSchedBreakdown] = useState<Record<string, number>>({});
  const [viewMode, setViewMode] = useState<AccountViewMode>("flat");
  const [bindingUsageSlots, setBindingUsageSlots] = useState<BindingSlotsResponse["by_binding"]>({});
  const [bindingUsageLoading, setBindingUsageLoading] = useState(false);
  const [heatmapWeekOffset, setHeatmapWeekOffset] = useState(0);
  const [heatmapWeekLabel, setHeatmapWeekLabel] = useState("");
  const [heatmapWeekdayLabels, setHeatmapWeekdayLabels] = useState(["一", "二", "三", "四", "五", "六", "日"]);
  const [heatmapDayLabels, setHeatmapDayLabels] = useState<string[]>([]);
  const [heatmapTimezone, setHeatmapTimezone] = useState<HeatmapTimezone>("Asia/Shanghai");
  const [heatmapTimezoneLabel, setHeatmapTimezoneLabel] = useState("");
  const [refreshingTokens, setRefreshingTokens] = useState<Set<string>>(new Set());
  const [sortKeys, setSortKeys] = useState<SortKey[]>([{ key: "created_at", dir: "desc" }]);
  const [nurturePresets, setNurturePresets] = useState<IpNurturePreset[]>([]);
  const [nurtureBindings, setNurtureBindings] = useState<Record<string, IpNurtureBinding>>({});
  const [bindingSaveBusy, setBindingSaveBusy] = useState<Set<string>>(new Set());
  const [weightEdit, setWeightEdit] = useState<{ key: string; presetId: string; weights: number[][] } | null>(null);
  const [editingAccount, setEditingAccount] = useState<Account | null>(null);
  const bindingUsageCacheRef = useRef(new Map<string, BindingSlotsResponse>());

  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  const applyBindingUsageResponse = (res: BindingSlotsResponse) => {
    setBindingUsageSlots(res.by_binding ?? {});
    setHeatmapWeekLabel(String(res.week_label || ""));
    setHeatmapWeekdayLabels(res.weekday_labels ?? ["一", "二", "三", "四", "五", "六", "日"]);
    setHeatmapDayLabels(res.day_labels ?? []);
    setHeatmapTimezoneLabel(String(res.timezone_label || ""));
    if (res.timezone === "Asia/Shanghai" || res.timezone === "Asia/Singapore") {
      setHeatmapTimezone(res.timezone);
    }
    setHeatmapWeekOffset(Number(res.week_offset ?? 0));
  };

  const loadBindingUsageSlots = useCallback(
    async (weekOffset = heatmapWeekOffset, timezone: HeatmapTimezone = heatmapTimezone, options?: { force?: boolean }) => {
      const cacheKey = `${weekOffset}:${timezone}`;
      const cached = bindingUsageCacheRef.current.get(cacheKey);
      if (cached && !options?.force) {
        applyBindingUsageResponse(cached);
        return;
      }
      setBindingUsageLoading(true);
      try {
        const res = await accountsApi.bindingSlots({ week_offset: weekOffset, timezone });
        bindingUsageCacheRef.current.set(cacheKey, res);
        applyBindingUsageResponse(res);
      } catch {
        setBindingUsageSlots({});
      } finally {
        setBindingUsageLoading(false);
      }
    },
    [heatmapWeekOffset, heatmapTimezone],
  );

  const loadNurtureData = useCallback(async () => {
    try {
      const [presetsRes, bindingsRes] = await Promise.all([opsApi.ipNurturePresets(), opsApi.ipNurtureBindings()]);
      setNurturePresets(Array.isArray(presetsRes.presets) ? presetsRes.presets : []);
      const map: Record<string, IpNurtureBinding> = {};
      for (const [key, item] of Object.entries(bindingsRes.bindings || {})) {
        if (item?.binding_key || key) {
          map[item?.binding_key || key] = item;
        }
      }
      setNurtureBindings(map);
    } catch {
      // optional
    }
  }, []);

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
  }, []);

  useEffect(() => {
    void loadNurtureData();
  }, [loadNurtureData]);

  useEffect(() => {
    if (viewMode === "grouped") {
      void loadBindingUsageSlots(heatmapWeekOffset, heatmapTimezone);
    }
  }, [viewMode, heatmapWeekOffset, heatmapTimezone, loadBindingUsageSlots]);

  const load = useCallback(
    async (options?: { force?: boolean; background?: boolean; page?: number }) => {
      const targetPage = options?.page ?? page;
      if (!options?.background) setLoading(true);
      setError("");
      try {
        const offset = (targetPage - 1) * PAGE_SIZE;
        const list = await fetchWithCache(
          `accounts:list:${offset}:${PAGE_SIZE}`,
          () => accountsApi.list({ offset, limit: PAGE_SIZE }),
          45_000,
          { force: options?.force },
        );
        setItems(list.data.items ?? []);
        setStats(list.data.stats);
        setTotal(list.data.total ?? list.data.items?.length ?? 0);
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

  const sorted = useMemo(() => sortAccounts(items, sortKeys), [items, sortKeys]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return sorted;
    return sorted.filter((row) => (row.email ?? "").toLowerCase().includes(q));
  }, [sorted, search]);

  const abnormalTokens = useMemo(
    () => filtered.filter((r) => r.status === "异常").map((r) => r.access_token),
    [filtered],
  );

  const selectedAccount = useMemo(() => {
    if (selected.size !== 1) return null;
    const token = Array.from(selected)[0];
    return filtered.find((r) => r.access_token === token) ?? null;
  }, [selected, filtered]);

  const cards = statCards(stats);

  const toggleSort = (key: string, shiftKey: boolean) => {
    setSortKeys((prev) => {
      const existing = prev.find((k) => k.key === key);
      if (shiftKey) {
        if (existing) {
          return prev.map((k) => (k.key === key ? { ...k, dir: k.dir === "asc" ? "desc" : "asc" } : k));
        }
        return [...prev, { key, dir: "asc" }];
      }
      if (existing && prev.length === 1) {
        return [{ key, dir: existing.dir === "asc" ? "desc" : "asc" }];
      }
      return [{ key, dir: "desc" }];
    });
  };

  const onRefresh = () => {
    invalidateCache("accounts:");
    setActivityRefreshToken((n) => n + 1);
    void load({ force: true, page });
  };

  const onReloadStorage = async () => {
    try {
      await accountsApi.reloadFromStorage();
      invalidateCache("accounts:");
      setActivityRefreshToken((n) => n + 1);
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
      selected.size > 0 ? Array.from(selected) : filtered.filter((r) => r.status === "异常").map((r) => r.access_token);
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

  const onDeleteTokens = async (tokens: string[]) => {
    if (tokens.length === 0) return;
    if (!window.confirm(`确认删除 ${tokens.length} 个账号？此操作不可撤销。`)) return;
    setDeleting(true);
    setError("");
    try {
      await accountsApi.deleteMany(tokens);
      setSelected((prev) => {
        const next = new Set(prev);
        for (const token of tokens) next.delete(token);
        return next;
      });
      invalidateCache("accounts:");
      setActivityRefreshToken((n) => n + 1);
      await load({ force: true, page });
      alert(`已删除 ${tokens.length} 个账号`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除失败");
    } finally {
      setDeleting(false);
    }
  };

  const onBulkQuotaWindowPrime = async (accessTokens: string[]) => {
    if (accessTokens.length === 0) {
      alert("请先选择要预热的账号");
      return;
    }
    if (accessTokens.length > MAX_PRIME) {
      alert(`单次最多预热 ${MAX_PRIME} 个账号`);
      return;
    }
    try {
      await accountsApi.primeQuotaWindow({ access_tokens: accessTokens, mode: "manual" });
      invalidateCache("accounts:");
      await load({ force: true, page });
      alert(`已入队 ${accessTokens.length} 个账号进行窗口预热`);
    } catch (err) {
      alert(err instanceof Error ? err.message : "批量窗口预热失败");
    }
  };

  const onPrimeAccount = async (account: Account) => {
    const email = String(account.email || "").trim();
    try {
      await accountsApi.primeQuotaWindow({ access_tokens: [account.access_token], mode: "manual" });
      invalidateCache("accounts:");
      await load({ force: true, page });
      alert(`${email || "账号"} 已加入窗口预热队列`);
    } catch (err) {
      alert(err instanceof Error ? err.message : "窗口预热失败");
    }
  };

  const onDialogueAccount = async (account: Account) => {
    const email = String(account.email || "").trim();
    if (!email) {
      alert("该账号无邮箱，无法定向对话");
      return;
    }
    try {
      const result = await opsApi.nurtureProcessOne({ email, source: "accounts_ui" });
      const ms = Number(result.latency_ms || 0);
      alert(`真实对话完成 ${email} · ${(ms / 1000).toFixed(1)}s · 输出 ${result.chars_out ?? 0} 字`);
      const usage = await accountsApi.usageRecent(USAGE_DAYS).catch(() => null);
      if (usage?.by_email) {
        setUsageByEmail(usage.by_email);
        if (usage.dates?.length) setUsageDates(usage.dates);
      }
    } catch (err) {
      alert(err instanceof Error ? err.message : "定向对话失败");
    }
  };

  const saveBindingPreset = async (bindingKey: string, presetId: string, customMatrix?: number[][]) => {
    setBindingSaveBusy((prev) => new Set(prev).add(bindingKey));
    try {
      const result = await opsApi.saveIpNurtureBinding({
        binding_key: bindingKey,
        preset_id: presetId,
        custom_matrix: customMatrix,
      });
      if (result.binding_key) {
        setNurtureBindings((prev) => ({ ...prev, [result.binding_key]: result }));
      }
      alert("IP 养号日历已保存");
    } catch (err) {
      alert(err instanceof Error ? err.message : "保存绑定日历失败");
    } finally {
      setBindingSaveBusy((prev) => {
        const next = new Set(prev);
        next.delete(bindingKey);
        return next;
      });
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

  const onRefreshSingleAccount = async (token: string) => {
    setRefreshingTokens((prev) => new Set(prev).add(token));
    setError("");
    try {
      const { progress_id } = await accountsApi.refresh([token]);
      await pollProgress(accountsApi.refreshProgress, progress_id);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "刷新失败");
    } finally {
      setRefreshingTokens((prev) => {
        const next = new Set(prev);
        next.delete(token);
        return next;
      });
    }
  };

  const onReloginSingleAccount = async (token: string) => {
    setRelogging(true);
    setError("");
    try {
      const { progress_id } = await accountsApi.reLogin([token]);
      await pollProgress(accountsApi.reLoginProgress, progress_id);
      invalidateCache("accounts:");
      await load({ force: true, page });
    } catch (err) {
      setError(err instanceof Error ? err.message : "重登失败");
    } finally {
      setRelogging(false);
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

      <div className="mt-4">
        <AccountsActivityPanels refreshToken={activityRefreshToken} />
      </div>

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

      <RefreshAllPanel onCompleted={() => void load({ force: true, page })} />

      <OutlookRecoveryPanel
        selectedAccount={selectedAccount}
        onCompleted={() => void load({ force: true, page })}
      />

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

        <div className="flex flex-wrap items-center gap-2 border-b border-[var(--neo-border)] px-4 py-2">
          <Button
            size="sm"
            variant="ghost"
            className="h-8 gap-1.5 text-orange-600"
            disabled={selected.size === 0}
            onClick={() => void onBulkQuotaWindowPrime(Array.from(selected))}
          >
            <Sun className="size-3.5" />
            批量窗口预热
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-8 gap-1.5 text-rose-500"
            disabled={abnormalTokens.length === 0 || deleting}
            onClick={() => void onDeleteTokens(abnormalTokens)}
          >
            {deleting ? <LoaderCircle className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
            移除异常账号
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-8 gap-1.5 text-rose-500"
            disabled={selected.size === 0 || deleting}
            onClick={() => void onDeleteTokens(Array.from(selected))}
          >
            {deleting ? <LoaderCircle className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
            删除所选
          </Button>
        </div>

        <AccountsDataTable
          rows={filtered}
          allRows={items}
          startIndex={(page - 1) * PAGE_SIZE}
          viewMode={viewMode}
          onViewModeChange={setViewMode}
          selected={selected}
          onSelectedChange={setSelected}
          sortKeys={sortKeys}
          onSort={toggleSort}
          usageDates={usageDates}
          usageByEmail={usageByEmail}
          bindingUsageSlots={bindingUsageSlots ?? {}}
          heatmapWeekOffset={heatmapWeekOffset}
          heatmapWeekLabel={heatmapWeekLabel}
          heatmapWeekdayLabels={heatmapWeekdayLabels}
          heatmapDayLabels={heatmapDayLabels}
          heatmapTimezone={heatmapTimezone}
          heatmapTimezoneLabel={heatmapTimezoneLabel}
          bindingUsageLoading={bindingUsageLoading}
          onHeatmapWeekOffsetChange={(offset) => {
            setHeatmapWeekOffset(offset);
            void loadBindingUsageSlots(offset, heatmapTimezone, { force: true });
          }}
          onHeatmapTimezoneChange={(tz) => {
            setHeatmapTimezone(tz);
            void loadBindingUsageSlots(heatmapWeekOffset, tz, { force: true });
          }}
          schedulingBusy={schedulingBusy}
          bulkScheduling={bulkScheduling}
          refreshingTokens={refreshingTokens}
          isRefreshing={refreshing}
          nurturePresets={nurturePresets}
          nurtureBindings={nurtureBindings}
          bindingSaveBusy={bindingSaveBusy}
          onNurturePresetChange={(bindingKey, presetId) => void saveBindingPreset(bindingKey, presetId)}
          onEditWeights={(bindingKey, presetId, weights) => setWeightEdit({ key: bindingKey, presetId, weights })}
          onToggleScheduling={(account) => void onToggleScheduling(account)}
          onRefreshAccount={(token) => void onRefreshSingleAccount(token)}
          onReloginAccount={(token) => void onReloginSingleAccount(token)}
          onPrimeAccount={(account) => void onPrimeAccount(account)}
          onDialogueAccount={(account) => void onDialogueAccount(account)}
          onEditAccount={setEditingAccount}
          onDeleteAccount={(token) => void onDeleteTokens([token])}
        />
      </ElevatedCard>

      <NurtureWeightDialog
        open={Boolean(weightEdit)}
        onOpenChange={(open) => {
          if (!open) setWeightEdit(null);
        }}
        bindingKey={weightEdit?.key ?? ""}
        presets={nurturePresets}
        initialWeights={weightEdit?.weights ?? []}
        onSave={(matrix) => saveBindingPreset(weightEdit?.key ?? "", weightEdit?.presetId ?? nurturePresets[0]?.id ?? "", matrix)}
      />

      <AccountEditDialog
        open={Boolean(editingAccount)}
        account={editingAccount}
        onOpenChange={(open) => {
          if (!open) setEditingAccount(null);
        }}
        onSaved={() => void load({ force: true, page })}
      />
    </PageShell>
  );
}
