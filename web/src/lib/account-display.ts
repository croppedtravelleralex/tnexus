import type { Account } from "@/lib/api";
import { summarizeCfDay, type CfDayPoint } from "@/components/accounts/CfStatusLight";
import type { EgressDayPoint } from "@/components/accounts/EgressDriftLights";

const EGRESS_STATUS_PRIORITY: Record<string, number> = {
  error: 3,
  warn: 2,
  ok: 1,
  none: 0,
};

const CF_STATUS_PRIORITY: Record<string, number> = {
  error: 3,
  warn: 2,
  ok: 1,
  none: 0,
};

export function maskToken(token?: string) {
  if (!token) return "—";
  if (token.length <= 18) return token;
  return `${token.slice(0, 16)}...${token.slice(-8)}`;
}

export function displayAccountType(account: Account) {
  return account.type || "free";
}

export function displayAccountSource(account: Account) {
  const source = String(account.source_type || "").trim().toLowerCase();
  return source || "web";
}

export function proxyDisplay(account: Account) {
  const egressIp = String(account.proxy_egress_ip ?? "").trim();
  const rawProxy = String(account.proxy ?? "").trim();
  let endpoint = "默认出口";
  if (egressIp) {
    endpoint = egressIp;
  } else if (rawProxy) {
    try {
      const parsed = new URL(rawProxy);
      endpoint = parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname;
    } catch {
      endpoint = rawProxy.replace(/^[a-z]+:\/\//i, "").replace(/^[^@]+@/, "").split("/")[0] || "账号代理";
    }
  }
  const provider = String(account.proxy_provider ?? "").trim();
  return {
    endpoint,
    provider,
    detail: provider || (rawProxy ? "账号级代理" : "运行时默认"),
  };
}

export function bindingLabelForAccount(account: Account) {
  return proxyDisplay(account).endpoint;
}

export function egressDaysForAccount(account: Account): EgressDayPoint[] {
  const today = new Date();
  const dates: string[] = [];
  for (let i = 6; i >= 0; i -= 1) {
    const d = new Date(today);
    d.setDate(today.getDate() - i);
    dates.push(d.toISOString().slice(0, 10));
  }
  const byDate = new Map<string, EgressDayPoint>();
  for (const row of account.egress_daily || []) {
    if (!row || typeof row !== "object") continue;
    const date = String(row.date || "").slice(0, 10);
    if (!date) continue;
    byDate.set(date, {
      date,
      status: String(row.status || "ok"),
      ip: String(row.ip || "") || undefined,
    });
  }
  return dates.map((date) => byDate.get(date) || { date, status: "none" });
}

export function aggregateEgressDays(accounts: Account[]) {
  if (!accounts.length) return [];
  const base = egressDaysForAccount(accounts[0]);
  return base.map((day, idx) => {
    let worst = day;
    let worstPri = EGRESS_STATUS_PRIORITY[String(day.status || "none").toLowerCase()] ?? 0;
    for (let i = 1; i < accounts.length; i += 1) {
      const other = egressDaysForAccount(accounts[i])[idx];
      const pri = EGRESS_STATUS_PRIORITY[String(other.status || "none").toLowerCase()] ?? 0;
      if (pri > worstPri) {
        worst = other;
        worstPri = pri;
      }
    }
    return worst;
  });
}

export function aggregateCfDays(accounts: Account[]) {
  if (!accounts.length) return [];
  const base = accounts[0].cf_daily?.length
    ? (() => {
        const today = new Date();
        const dates: string[] = [];
        for (let i = 6; i >= 0; i -= 1) {
          const d = new Date(today);
          d.setDate(today.getDate() - i);
          dates.push(d.toISOString().slice(0, 10));
        }
        const byDate = new Map<string, CfDayPoint>();
        for (const row of accounts[0].cf_daily || []) {
          const date = String(row.date || "").slice(0, 10);
          if (date) byDate.set(date, row);
        }
        return dates.map((date) => byDate.get(date) || { date, ok: 0, cf: 0, image_fail: 0 });
      })()
    : [];
  return base.map((day, idx) => {
    let worst = day;
    let worstPri = CF_STATUS_PRIORITY[summarizeCfDay(day).status] ?? 0;
    for (let i = 1; i < accounts.length; i += 1) {
      const rows = accounts[i].cf_daily || [];
      const other = rows[idx] || { date: day.date, ok: 0, cf: 0, image_fail: 0 };
      const pri = CF_STATUS_PRIORITY[summarizeCfDay(other).status] ?? 0;
      if (pri > worstPri) {
        worst = other;
        worstPri = pri;
      }
    }
    return worst;
  });
}

export function formatRestoreAtDetail(value?: string | null, account?: Account) {
  if (!value) {
    return { absolute: "—", relative: "", label: "" };
  }
  const date = new Date(value.endsWith("Z") || value.includes("+") ? value : `${value}Z`);
  if (Number.isNaN(date.getTime())) {
    return { absolute: value, relative: "", label: "" };
  }
  const diffMs = Math.max(0, date.getTime() - Date.now());
  const totalHours = Math.ceil(diffMs / (1000 * 60 * 60));
  const days = Math.floor(totalHours / 24);
  const hours = totalHours % 24;
  const relative = diffMs > 0 ? `剩余 ${days}d ${hours}h` : "已到恢复时间";
  const pad = (num: number) => String(num).padStart(2, "0");
  const absolute = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  const quota = Number(account?.quota ?? 0);
  const label = account && quota > 0 ? "窗口结束" : account ? "预计恢复" : "";
  return { absolute, relative, label };
}

export function formatCreatedAt(raw?: string | null) {
  if (!raw) return "—";
  try {
    const d = new Date(raw.endsWith("Z") || raw.includes("+") ? raw : `${raw}Z`);
    if (Number.isNaN(d.getTime())) return String(raw).slice(0, 10);
    return d.toLocaleDateString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit" });
  } catch {
    return String(raw).slice(0, 10);
  }
}

export function formatImageDateTime(raw?: string | null) {
  if (!raw) return "";
  try {
    const d = new Date(raw.endsWith("Z") || raw.includes("+") ? raw : `${raw}Z`);
    if (Number.isNaN(d.getTime())) return "";
    return d.toLocaleString("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return "";
  }
}
