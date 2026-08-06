// Grok 管理 API 客户端（grok-admin crate，G4-P2）。
//
// 注意：grok-admin 使用独立的 Bearer JWT（HS256），与 TNexus 会话登录（cookie）
// 是两套体系。这里约定 token 存 localStorage `tnexus_grok_admin_token`；
// 页面在无 token 时做只读降级提示。
// TODO：统一登录体系后（G6），此处改为复用会话自动换取 grok-admin token。

const API_BASE = process.env.NEXT_PUBLIC_API_BASE ?? "http://localhost:9000";

/** grok-admin HTTP 挂载点；默认与 tnexus-api 同源，可用 NEXT_PUBLIC_GROK_ADMIN_BASE 覆盖 */
export const GROK_ADMIN_BASE =
  process.env.NEXT_PUBLIC_GROK_ADMIN_BASE ?? API_BASE;

export const GROK_ADMIN_TOKEN_KEY = "tnexus_grok_admin_token";

/** 账号列表视图（对齐 grok-admin `AccountView`，snake_case） */
export type GrokAccountView = {
  id: number;
  provider: string;
  name: string;
  enabled: boolean;
  auth_status: string;
  priority: number;
  observed_model: string | null;
  max_concurrent: number;
  failure_count: number;
  cooldown_until: string | null;
  last_error: string | null;
  created_at: string | null;
  updated_at: string | null;
};

/** 分页列表（对齐 grok-admin `AccountPage`） */
export type GrokAccountPage = {
  items: GrokAccountView[];
  page: number;
  page_size: number;
  total: number;
};

/** 额度窗口（对齐 grok-domain `QuotaWindow`，snake_case） */
export type GrokQuotaWindow = {
  account_id: number;
  mode: string;
  remaining: number;
  total: number;
  reset_at: string | null;
  synced_at: string | null;
  source: string;
  updated_at: string;
};

/** 模型状态（对齐 grok-domain `ModelState`） */
export type GrokModelState = {
  account_id: number;
  upstream_model: string;
  status: string;
  reason: string | null;
  consecutive_failures: number;
  last_attempt_at: string | null;
  last_success_at: string | null;
  cooldown_until: string | null;
  updated_at: string;
};

/** 审计条目（对齐 grok-admin `AuditEntryView`，snake_case） */
export type GrokAuditEntry = {
  id: number;
  account_id: number | null;
  provider: string | null;
  upstream_model: string | null;
  status: number;
  /** success / error */
  outcome: string;
  latency_ms: number;
  created_at: string;
};

/** 审计分页（对齐 grok-admin `request-audits` 响应） */
export type GrokAuditPage = {
  items: GrokAuditEntry[];
  page: number;
  pageSize: number;
  total: number;
};

/** 审计汇总（对齐 grok-admin `AuditSummaryView`） */
export type GrokAuditSummary = {
  total: number;
  requests_24h: number;
  succeeded_24h: number;
  failed_24h: number;
  success_rate_24h: number;
};

/** 账号详情（账号 + 额度窗口 + 模型状态，对齐 `AccountDetail`） */
export type GrokAccountDetail = GrokAccountView & {
  quota_windows: GrokQuotaWindow[];
  model_states: GrokModelState[];
};

/** 更新输入（对齐 grok-admin `UpdateAccountInput`） */
export type GrokUpdateAccountInput = {
  enabled?: boolean;
  auth_status?: string;
  priority?: number;
  cooldown_until?: string | null;
};

/** 池规模汇总（对齐 grok-admin `AccountSummary`） */
export type GrokAccountSummary = {
  total: number;
  available: number;
  cooldown: number;
  reauth_required: number;
  disabled: number;
  probing: number;
  quota_exhausted: number;
  by_provider: Record<string, GrokProviderSummary>;
};

export type GrokProviderSummary = {
  total: number;
  available: number;
  cooldown: number;
  reauth_required: number;
  disabled: number;
  probing: number;
  quota_exhausted: number;
};

export function getGrokAdminToken(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(GROK_ADMIN_TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setGrokAdminToken(token: string) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(GROK_ADMIN_TOKEN_KEY, token);
  } catch {
    // ignore quota / private mode
  }
}

export function clearGrokAdminToken() {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(GROK_ADMIN_TOKEN_KEY);
  } catch {
    // ignore
  }
}

export const grokAdminApi = {
  listAccounts: async (
    token: string,
    params?: { page?: number; pageSize?: number; provider?: string },
  ): Promise<GrokAccountPage> => {
    const q = new URLSearchParams();
    if (params?.page != null) q.set("page", String(params.page));
    if (params?.pageSize != null) q.set("pageSize", String(params.pageSize));
    if (params?.provider) q.set("provider", params.provider);
    const query = q.toString();
    let res: Response;
    try {
      res = await fetch(`${GROK_ADMIN_BASE}/admin/accounts${query ? `?${query}` : ""}`, {
        headers: { Authorization: `Bearer ${token}` },
      });
    } catch {
      throw new Error(`无法连接 grok-admin（${GROK_ADMIN_BASE}）`);
    }
    if (res.status === 401) {
      throw new Error("管理员会话无效或已过期（401）");
    }
    if (!res.ok) {
      throw new Error(`grok-admin 返回 ${res.status}: ${res.statusText}`);
    }
    return res.json() as Promise<GrokAccountPage>;
  },

  getSummary: async (token: string): Promise<GrokAccountSummary> => {
    return grokAdminGet<GrokAccountSummary>(token, "/admin/accounts/summary");
  },

  getDetail: async (token: string, id: number): Promise<GrokAccountDetail> => {
    return grokAdminGet<GrokAccountDetail>(token, `/admin/accounts/${id}`);
  },

  getQuotaWindows: async (token: string, id: number): Promise<GrokQuotaWindow[]> => {
    return grokAdminGet<GrokQuotaWindow[]>(token, `/admin/accounts/${id}/quota`);
  },

  getModelStates: async (token: string, id: number): Promise<GrokModelState[]> => {
    return grokAdminGet<GrokModelState[]>(token, `/admin/accounts/${id}/model-states`);
  },

  updateAccount: async (
    token: string,
    id: number,
    input: GrokUpdateAccountInput,
  ): Promise<GrokAccountView> => {
    const res = await grokAdminFetch(token, `/admin/accounts/${id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(input),
    });
    return res.json() as Promise<GrokAccountView>;
  },

  /** 运维动作：billing 刷新 / quota 刷新 / token 刷新 / 重登（后端无数据不报错） */
  refreshAccount: async (token: string, id: number, kind: "billing" | "quota" | "token" | "reauth") => {
    const res = await grokAdminFetch(
      token,
      `/admin/accounts/${id}/refresh-${kind}`,
      { method: "POST" },
    );
    return res.json();
  },

  deleteAccount: async (token: string, id: number) => {
    const res = await grokAdminFetch(token, `/admin/accounts/${id}`, { method: "DELETE" });
    return res.json();
  },

  /** 审计流水（分页，按时间倒序；对齐 grok-admin `request-audits`） */
  listAudits: async (
    token: string,
    params?: { page?: number; pageSize?: number },
  ): Promise<GrokAuditPage> => {
    const q = new URLSearchParams();
    if (params?.page != null) q.set("page", String(params.page));
    if (params?.pageSize != null) q.set("pageSize", String(params.pageSize));
    const query = q.toString();
    return grokAdminGet<GrokAuditPage>(
      token,
      `/admin/request-audits${query ? `?${query}` : ""}`,
    );
  },

  /** 审计汇总（近 24h 成功率等） */
  getAuditSummary: async (token: string): Promise<GrokAuditSummary> => {
    return grokAdminGet<GrokAuditSummary>(token, "/admin/request-audits/summary");
  },
};

/** 拉取多页审计，直到达到 `limit` 条（客户端聚合用；数据不足返回已有条目）。 */
export async function grokAdminListAuditsUpTo(
  token: string,
  limit: number,
): Promise<GrokAuditEntry[]> {
  const pageSize = Math.min(100, Math.max(1, limit));
  const first = await grokAdminApi.listAudits(token, { page: 1, pageSize });
  const items = [...(first.items ?? [])];
  const total = first.total ?? items.length;
  let page = 2;
  while (items.length < Math.min(limit, total) && page <= 5) {
    const next = await grokAdminApi.listAudits(token, { page, pageSize });
    items.push(...(next.items ?? []));
    if ((next.items ?? []).length === 0) break;
    page += 1;
  }
  return items.slice(0, limit);
}

async function grokAdminFetch(
  token: string,
  path: string,
  init?: RequestInit,
): Promise<Response> {
  let res: Response;
  try {
    res = await fetch(`${GROK_ADMIN_BASE}${path}`, {
      ...init,
      headers: { Authorization: `Bearer ${token}`, ...(init?.headers ?? {}) },
    });
  } catch {
    throw new Error(`无法连接 grok-admin（${GROK_ADMIN_BASE}）`);
  }
  if (res.status === 401) {
    throw new Error("管理员会话无效或已过期（401）");
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`grok-admin 返回 ${res.status}: ${text || res.statusText}`);
  }
  return res;
}

async function grokAdminGet<T>(token: string, path: string): Promise<T> {
  const res = await grokAdminFetch(token, path);
  return res.json() as Promise<T>;
}
