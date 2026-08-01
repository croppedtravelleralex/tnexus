import type { Account } from "@/lib/api";

export type ImageQuotaState =
  | "unlimited"
  | "unknown"
  | "ready"
  | "unverified"
  | "stale"
  | "blocked"
  | "refresh_pending"
  | "exhausted";

const QUOTA_STATE_LABEL: Record<ImageQuotaState, string> = {
  unlimited: "无限额",
  unknown: "未核对",
  ready: "可用",
  unverified: "待核对",
  stale: "待刷新",
  blocked: "不可调度",
  refresh_pending: "待恢复",
  exhausted: "已耗尽",
};

export function isUnlimitedImageQuotaAccount(account: Pick<Account, "type">) {
  const type = String(account.type || "").trim().toLowerCase();
  return type === "pro" || type === "prolite";
}

export function isManualSchedulingEnabled(account: Pick<Account, "panda_receive_state">) {
  const receive = String(account.panda_receive_state ?? "").trim().toLowerCase();
  if (receive === "rejected" || receive === "identity_isolated") return false;
  return (
    !receive ||
    receive === "verified_ready" ||
    receive === "verified" ||
    receive === "local_verified"
  );
}

export function accountImageQuotaState(account: Account): ImageQuotaState {
  const state = String(account.image_quota_state || "").trim().toLowerCase();
  if (state in QUOTA_STATE_LABEL) return state as ImageQuotaState;
  if (isUnlimitedImageQuotaAccount(account)) return "unlimited";
  if (account.image_quota_unknown) return "unknown";
  if (typeof account.available_image_quota === "number" && account.available_image_quota > 0) return "ready";
  if (Number(account.quota || 0) > 0) return "blocked";
  return "exhausted";
}

export function formatAccountQuotaValue(account: Account) {
  if (isUnlimitedImageQuotaAccount(account) || accountImageQuotaState(account) === "unlimited") return "∞";
  if (account.image_quota_unknown || accountImageQuotaState(account) === "unknown") return "未知";
  return String(Math.max(0, Number(account.quota || 0)));
}

export function accountQuotaBadgeVariant(account: Account): "success" | "info" | "warning" | "secondary" | "danger" {
  const inSchedule = isManualSchedulingEnabled(account) && account.status === "正常";
  if (inSchedule) return "success";
  const state = accountImageQuotaState(account);
  if (state === "unlimited") return "info";
  if (state === "unverified" || state === "refresh_pending") return "warning";
  return "secondary";
}

export function formatAccountQuotaHint(account: Account) {
  const state = accountImageQuotaState(account);
  const label = QUOTA_STATE_LABEL[state];
  const cached = Math.max(0, Number(account.quota || 0));
  if (state === "ready" || state === "unverified") {
    const schedulable = Math.max(0, Number(account.available_image_quota ?? account.quota ?? 0));
    return `生图可调度 ${schedulable}（账面 ${cached}，${label}）`;
  }
  return label;
}

export function formatQuotaRefreshAgeFromIso(raw?: string | null) {
  if (!raw) return "未核对";
  const at = new Date(raw.endsWith("Z") || raw.includes("+") ? raw : `${raw}Z`);
  if (Number.isNaN(at.getTime())) return "未核对";
  const diffMin = Math.floor(Math.max(0, Date.now() - at.getTime()) / 60_000);
  if (diffMin < 1) return "1分钟内";
  if (diffMin < 60) return `${diffMin}分钟前`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 48) return `${diffHr}小时前`;
  return `${Math.floor(diffHr / 24)}天前`;
}

export function formatQuotaRefreshAge(account: Pick<Account, "last_quota_refresh_at">) {
  return formatQuotaRefreshAgeFromIso(account.last_quota_refresh_at);
}

export function formatRestoreAt(restoreAt?: string | null, account?: Account) {
  if (!restoreAt) return null;
  try {
    const end = new Date(restoreAt.endsWith("Z") || restoreAt.includes("+") ? restoreAt : `${restoreAt}Z`);
    if (Number.isNaN(end.getTime())) return restoreAt;
    const remainMs = end.getTime() - Date.now();
    if (remainMs <= 0) return { label: "可恢复", detail: end.toLocaleString("zh-CN") };
    const h = Math.floor(remainMs / 3_600_000);
    const d = Math.floor(h / 24);
    const rh = h % 24;
    const prefix = Number(account?.quota || 0) > 0 ? "窗口结束" : "预计恢复";
    return { label: prefix, detail: `剩余 ${d}d ${rh}h` };
  } catch {
    return { label: "恢复", detail: restoreAt };
  }
}
