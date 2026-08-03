"use client";

import {
  Ban,
  CheckCircle2,
  Copy,
  LoaderCircle,
  LogIn,
  MessageSquare,
  Pause,
  Pencil,
  Play,
  RefreshCw,
  Sun,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { useMemo } from "react";
import { AccountUsageHeatstrip } from "@/components/accounts/account-usage-heatstrip";
import { BindingActivityHeatmaps } from "@/components/accounts/BindingActivityHeatmaps";
import { BindingActivityHeatmapToolbar, type HeatmapTimezone } from "@/components/accounts/BindingActivityHeatmapToolbar";
import { normalizeBindingWeights } from "@/components/accounts/BindingSgHeatmap";
import { CfStatusLight, cfDaysForAccount } from "@/components/accounts/CfStatusLight";
import { EgressDriftLights } from "@/components/accounts/EgressDriftLights";
import { ScheduleCountdownIcons } from "@/components/accounts/ScheduleCountdownIcons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { Account, IpNurtureBinding, IpNurturePreset } from "@/lib/api";
import {
  collectAbnormalReasons,
  formatAbnormalSummary,
  formatRecoveryHint,
} from "@/lib/account-abnormal";
import {
  aggregateCfDays,
  aggregateEgressDays,
  bindingLabelForAccount,
  displayAccountSource,
  displayAccountType,
  egressDaysForAccount,
  formatCreatedAt,
  formatRestoreAtDetail,
  maskToken,
  proxyDisplay,
} from "@/lib/account-display";
import { bindingKeyForAccount } from "@/lib/binding-key";
import {
  accountQuotaBadgeVariant,
  formatAccountQuotaHint,
  formatAccountQuotaValue,
  formatQuotaRefreshAge,
} from "@/lib/image-quota";
import { cn } from "@/lib/utils";

export type AccountViewMode = "flat" | "grouped";
export type SortKey = { key: string; dir: "asc" | "desc" };

const TABLE_COLUMN_COUNT = 13;

const STATUS_META: Record<string, { badge: "success" | "warning" | "muted"; icon: typeof CheckCircle2 }> = {
  正常: { badge: "success", icon: CheckCircle2 },
  限流: { badge: "warning", icon: TriangleAlert },
  异常: { badge: "warning", icon: Ban },
  禁用: { badge: "muted", icon: Ban },
};

function isManualSchedulingEnabled(account: Account) {
  const receive = String(account.panda_receive_state ?? "").trim().toLowerCase();
  if (!receive) return true;
  return receive === "verified_ready" || receive === "verified" || receive === "local_verified";
}

function weightsForBinding(
  bindingKey: string,
  presets: IpNurturePreset[],
  bindings: Record<string, IpNurtureBinding>,
) {
  const binding = bindings[bindingKey];
  if (binding?.weights?.length) {
    return normalizeBindingWeights(binding.weights);
  }
  const preset = presets.find((item) => item.id === binding?.preset_id) || presets[0];
  const presetWeights = (preset as IpNurturePreset & { weights?: number[][] })?.weights;
  return normalizeBindingWeights(presetWeights || []);
}

type Props = {
  rows: Account[];
  allRows: Account[];
  startIndex: number;
  viewMode: AccountViewMode;
  onViewModeChange: (mode: AccountViewMode) => void;
  selected: Set<string>;
  onSelectedChange: (next: Set<string>) => void;
  sortKeys: SortKey[];
  onSort: (key: string, shiftKey: boolean) => void;
  usageDates: string[];
  usageByEmail: Record<string, Array<{ date: string; images: number; dialogues: number }>>;
  bindingUsageSlots: Record<string, Record<string, number[][]>>;
  heatmapWeekOffset: number;
  heatmapWeekLabel: string;
  heatmapWeekdayLabels: string[];
  heatmapDayLabels: string[];
  heatmapTimezone: HeatmapTimezone;
  heatmapTimezoneLabel: string;
  bindingUsageLoading: boolean;
  onHeatmapWeekOffsetChange: (offset: number) => void;
  onHeatmapTimezoneChange: (tz: HeatmapTimezone) => void;
  schedulingBusy: Set<string>;
  bulkScheduling: boolean;
  refreshingTokens: Set<string>;
  isRefreshing: boolean;
  nurturePresets: IpNurturePreset[];
  nurtureBindings: Record<string, IpNurtureBinding>;
  bindingSaveBusy?: Set<string>;
  onNurturePresetChange: (bindingKey: string, presetId: string) => void;
  onEditWeights: (bindingKey: string, presetId: string, weights: number[][]) => void;
  onToggleScheduling: (account: Account) => void;
  onRefreshAccount: (token: string) => void;
  onReloginAccount: (token: string) => void;
  onPrimeAccount: (account: Account) => void;
  onDialogueAccount: (account: Account) => void;
  onEditAccount: (account: Account) => void;
  onDeleteAccount: (token: string) => void;
};

export function AccountsDataTable({
  rows,
  allRows,
  startIndex,
  viewMode,
  onViewModeChange,
  selected,
  onSelectedChange,
  sortKeys,
  onSort,
  usageDates,
  usageByEmail,
  bindingUsageSlots,
  heatmapWeekOffset,
  heatmapWeekLabel,
  heatmapWeekdayLabels,
  heatmapDayLabels,
  heatmapTimezone,
  heatmapTimezoneLabel,
  bindingUsageLoading,
  onHeatmapWeekOffsetChange,
  onHeatmapTimezoneChange,
  schedulingBusy,
  bulkScheduling,
  refreshingTokens,
  isRefreshing,
  nurturePresets,
  nurtureBindings,
  bindingSaveBusy,
  onNurturePresetChange,
  onEditWeights,
  onToggleScheduling,
  onRefreshAccount,
  onReloginAccount,
  onPrimeAccount,
  onDialogueAccount,
  onEditAccount,
  onDeleteAccount,
}: Props) {
  const allPageSelected = rows.length > 0 && rows.every((r) => selected.has(r.access_token));

  const sortIndicator = (key: string) => {
    const idx = sortKeys.findIndex((k) => k.key === key);
    if (idx < 0) return "";
    const arrow = sortKeys[idx].dir === "asc" ? "↑" : "↓";
    return sortKeys.length > 1 ? `${arrow}${idx + 1}` : arrow;
  };

  const accountGroups = useMemo(() => {
    const map = new Map<string, { key: string; label: string; accounts: Account[] }>();
    for (const account of allRows) {
      const key = bindingKeyForAccount(account);
      const existing = map.get(key);
      if (existing) {
        existing.accounts.push(account);
      } else {
        map.set(key, { key, label: bindingLabelForAccount(account), accounts: [account] });
      }
    }
    return Array.from(map.values()).sort((a, b) => a.label.localeCompare(b.label, "zh-CN"));
  }, [allRows]);

  const tableBlocks = useMemo(() => {
    if (viewMode === "flat") {
      return rows.map((account, rowIndex) => ({
        kind: "account" as const,
        account,
        rowNo: startIndex + rowIndex + 1,
      }));
    }
    const blocks: Array<
      | { kind: "group"; key: string; label: string; accounts: Account[] }
      | { kind: "account"; account: Account; rowNo: number }
    > = [];
    const seenGroups = new Set<string>();
    let rowCounter = startIndex;
    for (const account of rows) {
      const key = bindingKeyForAccount(account);
      if (!seenGroups.has(key)) {
        seenGroups.add(key);
        const group = accountGroups.find((item) => item.key === key);
        if (group) {
          blocks.push({ kind: "group", key: group.key, label: group.label, accounts: group.accounts });
        }
      }
      rowCounter += 1;
      blocks.push({ kind: "account", account, rowNo: rowCounter });
    }
    return blocks;
  }, [viewMode, accountGroups, rows, startIndex]);

  const toggleSelectAll = () => {
    const next = new Set(selected);
    if (allPageSelected) {
      for (const row of rows) next.delete(row.access_token);
    } else {
      for (const row of rows) next.add(row.access_token);
    }
    onSelectedChange(next);
  };

  const renderAccountRow = (account: Account, rowNo: number) => {
    const status = STATUS_META[account.status] ?? STATUS_META["异常"];
    const StatusIcon = status.icon;
    const inSchedule = isManualSchedulingEnabled(account);
    const busy = schedulingBusy.has(account.access_token) || bulkScheduling;
    const rowRefreshing = isRefreshing || refreshingTokens.has(account.access_token);
    const proxy = proxyDisplay(account);
    const restore = formatRestoreAtDetail(account.restore_at, account);
    const primeState = String(account.quota_window_prime_state || "").toLowerCase();

    return (
      <tr key={account.access_token} className="border-t border-[var(--neo-border)] neo-row-hover text-sm text-stone-600">
        <td className="px-2 py-2 text-xs tabular-nums text-stone-400">{rowNo}</td>
        <td className="px-2 py-2">
          <input
            type="checkbox"
            checked={selected.has(account.access_token)}
            onChange={() => {
              const next = new Set(selected);
              if (next.has(account.access_token)) next.delete(account.access_token);
              else next.add(account.access_token);
              onSelectedChange(next);
            }}
            aria-label={`选择 ${account.email ?? "账号"}`}
          />
        </td>
        <td className="px-2 py-2">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <span className="font-medium tracking-tight text-stone-700">{maskToken(account.access_token)}</span>
              <button
                type="button"
                className="rounded-lg p-1 text-stone-400 transition hover:bg-stone-100 hover:text-stone-700"
                onClick={() => void navigator.clipboard.writeText(account.access_token)}
                title="复制 token"
              >
                <Copy className="size-3.5" />
              </button>
            </div>
            <div className="truncate text-xs text-stone-500">{account.email ?? "—"}</div>
          </div>
        </td>
        <td className="px-2 py-2">
          <div className="flex flex-col items-start gap-1">
            <Badge variant="muted" className="rounded-md capitalize">
              {displayAccountType(account)}
            </Badge>
            <Badge variant="muted" className="rounded-md border border-stone-200 bg-white text-stone-600">
              {displayAccountSource(account)}
            </Badge>
          </div>
        </td>
        <td className="px-2 py-2">
          <div className="max-w-52">
            <Badge variant={status.badge} className="inline-flex items-center gap-1 rounded-md px-2 py-1">
              <StatusIcon className="size-3.5" />
              {account.status}
            </Badge>
            {account.status === "异常" || account.status === "限流" ? (
              <div
                className="mt-1 space-y-0.5 text-[10px] leading-snug text-amber-800"
                title={collectAbnormalReasons(account).join("\n")}
              >
                <p className="line-clamp-2">{formatAbnormalSummary(account)}</p>
                <p className="text-amber-600/80">{formatRecoveryHint(account)}</p>
              </div>
            ) : null}
          </div>
        </td>
        <td className="px-2 py-2">
          <button
            type="button"
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium transition",
              inSchedule ? "bg-emerald-50 text-emerald-700 hover:bg-emerald-100" : "bg-amber-50 text-amber-700 hover:bg-amber-100",
            )}
            onClick={() => onToggleScheduling(account)}
            disabled={busy}
            title={inSchedule ? "当前在调度池；点击退出调度" : "当前隔离观察；点击进入调度"}
          >
            {busy ? <LoaderCircle className="size-3.5 animate-spin" /> : inSchedule ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
            {inSchedule ? "调度中" : "已隔离"}
          </button>
        </td>
        <td className="px-2 py-2">
          <AccountUsageHeatstrip
            days={
              usageByEmail[String(account.email ?? "").trim().toLowerCase()] ??
              usageDates.map((date) => ({ date, images: 0, dialogues: 0 }))
            }
          />
        </td>
        <td className="px-2 py-2">
          <div className="max-w-44 space-y-0.5 text-xs">
            <div className="flex min-w-0 items-baseline gap-1.5">
              <span className="truncate font-medium text-stone-700">{proxy.endpoint}</span>
              {proxy.provider ? <span className="shrink-0 text-stone-400">{proxy.provider}</span> : null}
            </div>
            <EgressDriftLights days={egressDaysForAccount(account)} />
            <CfStatusLight days={cfDaysForAccount(account)} />
          </div>
        </td>
        <td className="px-2 py-2 text-xs text-stone-500">{formatCreatedAt(account.created_at)}</td>
        <td className="px-2 py-2">
          <div className="flex items-center gap-1.5">
            <span className="shrink-0 text-[10px] text-stone-400" title={account.last_quota_refresh_at ? `额度核对：${account.last_quota_refresh_at}` : "尚未远程核对额度"}>
              {formatQuotaRefreshAge(account)}
            </span>
            <span title={formatAccountQuotaHint(account)}>
              <Badge variant={accountQuotaBadgeVariant(account)} className="rounded-md font-semibold tabular-nums">
                {formatAccountQuotaValue(account)}
              </Badge>
            </span>
          </div>
        </td>
        <td className="px-2 py-2 text-xs text-stone-500">
          <div className="space-y-0.5">
            {restore.label ? <div className="text-[10px] uppercase tracking-wide text-stone-400">{restore.label}</div> : null}
            <div className="flex items-center gap-1">
              {restore.relative ? <div className="font-medium text-stone-700">{restore.relative}</div> : null}
              <ScheduleCountdownIcons account={account} showText={false} />
            </div>
            <div>{restore.absolute}</div>
          </div>
        </td>
        <td className="px-2 py-2">
          <span
            className={cn("tabular-nums", (account.image_inflight ?? 0) > 0 ? "font-semibold text-amber-600" : "text-stone-400")}
            title={(account.image_inflight ?? 0) > 0 ? "当前有在途生图" : "无在途生图"}
          >
            {account.image_inflight ?? 0}
          </span>
        </td>
        <td className="px-2 py-2">
          <div className="flex items-center gap-0.5 text-stone-400">
            <ScheduleCountdownIcons account={account} showLazy={false} />
            <button
              type="button"
              className={cn(
                "rounded-lg p-1.5 transition",
                primeState === "done"
                  ? "text-emerald-600"
                  : ["pending", "running"].includes(primeState)
                    ? "text-orange-400"
                    : "hover:bg-orange-50 hover:text-orange-700",
              )}
              title={
                account.quota_window_prime_last_error
                  ? `预热失败：${account.quota_window_prime_last_error}`
                  : "打 1 张 256 最小图，钉住上游额度窗口"
              }
              disabled={["pending", "running", "done"].includes(primeState)}
              onClick={() => onPrimeAccount(account)}
            >
              <Sun className="size-3.5" />
            </button>
            <button
              type="button"
              className="rounded-lg p-1.5 transition hover:bg-sky-50 hover:text-sky-700"
              title="立即对该账号发起一条真实文本对话"
              onClick={() => onDialogueAccount(account)}
            >
              <MessageSquare className="size-3.5" />
            </button>
            <button
              type="button"
              className="rounded-lg p-1.5 transition hover:bg-stone-100 hover:text-stone-700"
              title="编辑账号"
              onClick={() => onEditAccount(account)}
            >
              <Pencil className="size-3.5" />
            </button>
            <button
              type="button"
              className="rounded-lg p-1.5 transition hover:bg-sky-50 hover:text-sky-700"
              title="刷新该账号额度"
              disabled={rowRefreshing}
              onClick={() => onRefreshAccount(account.access_token)}
            >
              {rowRefreshing ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            </button>
            {account.status === "异常" ? (
              <button
                type="button"
                className="rounded-lg p-1.5 transition hover:bg-amber-50 hover:text-amber-700"
                title="尝试重登恢复"
                onClick={() => onReloginAccount(account.access_token)}
              >
                <LogIn className="size-3.5" />
              </button>
            ) : null}
            <button
              type="button"
              className="rounded-lg p-1.5 transition hover:bg-rose-50 hover:text-rose-500"
              title="删除账号"
              onClick={() => onDeleteAccount(account.access_token)}
            >
              <Trash2 className="size-3.5" />
            </button>
          </div>
        </td>
      </tr>
    );
  };

  return (
    <div className="overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-[var(--neo-border)] px-4 py-3">
        <div className="inline-flex rounded-xl border border-stone-200 bg-white/85 p-0.5">
          <button
            type="button"
            className={cn("rounded-lg px-3 py-1.5 text-xs font-medium transition", viewMode === "flat" ? "bg-stone-900 text-white" : "text-stone-600 hover:bg-stone-100")}
            onClick={() => onViewModeChange("flat")}
          >
            平铺
          </button>
          <button
            type="button"
            className={cn("rounded-lg px-3 py-1.5 text-xs font-medium transition", viewMode === "grouped" ? "bg-stone-900 text-white" : "text-stone-600 hover:bg-stone-100")}
            onClick={() => onViewModeChange("grouped")}
          >
            按IP分组
          </button>
        </div>
      </div>

      {viewMode === "grouped" ? (
        <BindingActivityHeatmapToolbar
          className="mx-4 mb-2 mt-3"
          weekOffset={heatmapWeekOffset}
          weekLabel={heatmapWeekLabel}
          timezone={heatmapTimezone}
          timezoneLabel={heatmapTimezoneLabel}
          loading={bindingUsageLoading}
          onWeekOffsetChange={onHeatmapWeekOffsetChange}
          onTimezoneChange={onHeatmapTimezoneChange}
        />
      ) : null}

      <div className="overflow-x-auto">
        <table className="w-full min-w-[1400px] text-left text-sm">
          <thead className="neo-table-head text-[11px] uppercase tracking-wide text-stone-400">
            <tr>
              <th className="w-10 px-2 py-2">#</th>
              <th className="w-10 px-2 py-2">
                <input type="checkbox" checked={allPageSelected} onChange={toggleSelectAll} aria-label="全选本页" />
              </th>
              <th className="w-56 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("email", e.shiftKey)}>
                  Token / 邮箱 {sortIndicator("email")}
                </button>
              </th>
              <th className="w-24 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("type", e.shiftKey)}>
                  类型 {sortIndicator("type")}
                </button>
              </th>
              <th className="w-20 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("status", e.shiftKey)}>
                  状态 {sortIndicator("status")}
                </button>
              </th>
              <th className="w-20 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("scheduling", e.shiftKey)}>
                  调度 {sortIndicator("scheduling")}
                </button>
              </th>
              <th className="w-28 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("record", e.shiftKey)}>
                  记录 {sortIndicator("record")}
                </button>
              </th>
              <th className="w-40 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("proxy", e.shiftKey)}>
                  代理 / 出口 {sortIndicator("proxy")}
                </button>
              </th>
              <th className="w-28 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("created_at", e.shiftKey)}>
                  创建时间 {sortIndicator("created_at")}
                </button>
              </th>
              <th className="w-24 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("quota", e.shiftKey)}>
                  额度 {sortIndicator("quota")}
                </button>
              </th>
              <th className="w-36 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("window", e.shiftKey)}>
                  窗口/恢复 {sortIndicator("window")}
                </button>
              </th>
              <th className="w-14 px-2 py-2">
                <button type="button" className="hover:text-stone-700" onClick={(e) => onSort("inflight", e.shiftKey)}>
                  在途 {sortIndicator("inflight")}
                </button>
              </th>
              <th className="w-28 px-2 py-2">操作</th>
            </tr>
          </thead>
          <tbody>
            {tableBlocks.map((block) => {
              if (block.kind === "group") {
                const binding = nurtureBindings[block.key];
                const presetId = binding?.preset_id || nurturePresets[0]?.id || "";
                const weights = weightsForBinding(block.key, nurturePresets, nurtureBindings);
                const saving = bindingSaveBusy?.has(block.key) ?? false;
                return (
                  <tr key={`group-${block.key}`} className="border-t border-stone-200 bg-stone-50/90">
                    <td colSpan={TABLE_COLUMN_COUNT} className="px-3 py-2">
                      <div className="flex flex-wrap items-center gap-3">
                        <div className="min-w-28">
                          <div className="text-xs font-semibold text-stone-800">{block.label}</div>
                          <div className="text-[10px] text-stone-500">
                            {block.accounts.length} 账号 · {block.key.slice(0, 12)}
                            {block.key.length > 12 ? "…" : ""}
                          </div>
                        </div>
                        <EgressDriftLights days={aggregateEgressDays(block.accounts)} />
                        <CfStatusLight days={aggregateCfDays(block.accounts)} />
                        <div className="flex items-end gap-2">
                          <div className="space-y-1">
                            <div className="text-[10px] text-stone-500">养号日历</div>
                            <select
                              value={presetId}
                              disabled={saving || nurturePresets.length === 0}
                              onChange={(e) => onNurturePresetChange(block.key, e.target.value)}
                              className="neo-input h-7 w-32 rounded-lg px-2 text-xs"
                            >
                              {nurturePresets.map((preset) => (
                                <option key={preset.id} value={preset.id}>
                                  {preset.label}
                                </option>
                              ))}
                            </select>
                          </div>
                          <BindingActivityHeatmaps
                            matrices={bindingUsageSlots[block.key] || {}}
                            weekLabel={heatmapWeekLabel}
                            weekdayLabels={heatmapWeekdayLabels}
                            dayLabels={heatmapDayLabels}
                            timezoneLabel={heatmapTimezoneLabel}
                            compact
                          />
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-7 rounded-lg px-2 text-xs"
                            disabled={saving || nurturePresets.length === 0}
                            onClick={() => onEditWeights(block.key, presetId, weights)}
                          >
                            编辑权重
                          </Button>
                        </div>
                      </div>
                    </td>
                  </tr>
                );
              }
              return renderAccountRow(block.account, block.rowNo);
            })}
            {rows.length === 0 ? (
              <tr>
                <td colSpan={TABLE_COLUMN_COUNT} className="px-4 py-8 text-center text-[var(--neo-muted)]">
                  暂无账户
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}
