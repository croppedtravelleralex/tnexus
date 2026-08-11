/** Grok 号池 / 额度 / 探针状态汉化（DB 与 Go 侧英文枚举 → 控制台展示）。 */

const AUTH_STATUS: Record<string, string> = {
  active: "正常",
  restricted: "受限",
  banned: "封禁",
  reauth_required: "需重新登录",
  reauthRequired: "需重新登录",
  unknown: "未知",
};

const MODEL_STATUS: Record<string, string> = {
  available: "可用",
  quota_available: "额度可用",
  probe_lite_ok: "轻量探针通过",
  probe_ok: "探针通过",
  probe_chat_ok: "对话探针通过",
  unknown: "未知",
  soft_stop: "软停用",
  quota_exhausted: "额度耗尽",
  auth_failed: "认证失败",
  signature_failed: "签名失败",
  cooldown: "冷却中",
  probing: "探测中",
};

const QUOTA_MODE: Record<string, string> = {
  fast: "快速",
  auto: "自动",
  imagine: "生图",
  weekly: "周额度",
};

const QUOTA_SOURCE: Record<string, string> = {
  upstream: "上游同步",
  manual: "手动",
  probe: "探针",
  import: "导入",
};

const SUMMARY_BUCKET: Record<string, string> = {
  total: "全部",
  available: "可用",
  cooldown: "冷却",
  reauth_required: "需重登",
  disabled: "已禁用",
  probing: "探测中",
  quota_exhausted: "额度耗尽",
};

export function labelAuthStatus(status: string): string {
  return AUTH_STATUS[status] ?? status;
}

export function labelModelStatus(status: string): string {
  return MODEL_STATUS[status] ?? status;
}

export function labelQuotaMode(mode: string): string {
  return QUOTA_MODE[mode] ?? mode;
}

export function labelQuotaSource(source: string | null | undefined): string {
  if (!source) return "—";
  return QUOTA_SOURCE[source] ?? source;
}

export function labelSummaryBucket(key: string): string {
  return SUMMARY_BUCKET[key] ?? key;
}

/** 模型状态 + 可选 reason 汉化拼接 */
export function labelModelStatusLine(status: string, reason?: string | null): string {
  const base = labelModelStatus(status);
  if (!reason?.trim()) return base;
  const r = reason.trim();
  if (r === status || MODEL_STATUS[r]) {
    return `${base}（${labelModelStatus(r)}）`;
  }
  return `${base}（${r}）`;
}
