import type { Account } from "@/lib/api";

/** 收集账号异常/限流相关的可读原因（按优先级排序）。 */
export function collectAbnormalReasons(account: Account): string[] {
  const reasons: string[] = [];
  const push = (label: string, value?: string | null) => {
    const v = String(value ?? "").trim();
    if (v) reasons.push(`${label}: ${v}`);
  };

  if (!String(account.access_token ?? "").trim()) {
    reasons.push("缺少 access_token");
  }

  push("刷新失败", account.last_refresh_error);
  push("额度核对", account.last_quota_refresh_error);
  push("Panda 探测", account.panda_probe_last_error);
  push("Panda 校验", account.panda_verify_last_error);
  push("窗口预热", account.quota_window_prime_last_error);

  const receive = String(account.panda_receive_state ?? "").trim();
  if (receive && !["verified_ready", "verified", "local_verified"].includes(receive.toLowerCase())) {
    reasons.push(`接收状态: ${receive}`);
  }

  if (account.status === "限流" && account.restore_at) {
    reasons.push(`限流恢复: ${account.restore_at}`);
  }

  if (account.status === "异常" && account.restore_at) {
    reasons.push(`计划恢复: ${account.restore_at}`);
  }

  if (account.image_quota_state === "exhausted" || (account.quota ?? 0) <= 0) {
    if (account.image_quota_unknown) {
      reasons.push("图片额度未知");
    } else if ((account.quota ?? 0) <= 0) {
      reasons.push("图片额度为 0");
    }
  }

  return reasons;
}

export function formatAbnormalSummary(account: Account): string {
  const reasons = collectAbnormalReasons(account);
  if (reasons.length > 0) return reasons[0];
  if (account.status === "异常") return "状态异常，原因未记录";
  return "";
}

export function formatRecoveryHint(account: Account): string {
  if (account.status === "异常" || !String(account.access_token ?? "").trim()) {
    return "可尝试：单账号刷新额度 → 重登恢复";
  }
  if (account.status === "限流") {
    return "等待恢复时间或尝试窗口预热";
  }
  return "";
}
