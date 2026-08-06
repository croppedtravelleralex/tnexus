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
};
