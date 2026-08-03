const API_BASE = process.env.NEXT_PUBLIC_API_BASE ?? "http://localhost:9000";

/** 将 API 相对路径（如 /api/images/thumb/...）转为可加载的完整 URL */
export function apiAssetUrl(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  if (path.startsWith("http://") || path.startsWith("https://") || path.startsWith("data:")) {
    return path;
  }
  if (path.startsWith("/")) return `${API_BASE}${path}`;
  return path;
}

import type { ChatConversationState } from "@/lib/chat-conversations";
import type { Conversation, ConversationState } from "@/lib/conversations";
import type { GenConfig } from "@/lib/gen-config";

/** 账号级偏好（studio 布局已改为全局固定，不再写入 preferences） */
export type UserPreferences = Record<string, unknown>;

export type AccountStatus = "正常" | "限流" | "异常" | "禁用";

export type Account = {
  access_token: string;
  email?: string | null;
  type: string;
  status: AccountStatus;
  quota: number;
  image_schedulable?: boolean;
  image_quota_unknown?: boolean;
  image_quota_state?: string | null;
  available_image_quota?: number;
  panda_receive_state?: string | null;
  proxy?: string | null;
  proxy_egress_ip?: string | null;
  proxy_provider?: string | null;
  proxy_binding_hash?: string | null;
  source_type?: string | null;
  cf_daily?: Array<{ date?: string; ok?: number; cf?: number; image_fail?: number }>;
  egress_daily?: Array<{ date?: string; status?: string; ip?: string }>;
  success?: number;
  fail?: number;
  created_at?: string | null;
  restore_at?: string | null;
  image_inflight?: number;
  last_quota_refresh_at?: string | null;
  last_quota_refresh_error?: string | null;
  last_refresh_error?: string | null;
  panda_probe_last_error?: string | null;
  panda_verify_last_error?: string | null;
  lazy_refresh_in_sec?: number | null;
  lazy_refresh_eligible_at?: string | null;
  text_next_ok_in_sec?: number | null;
  text_next_ok_at?: string | null;
  quota_window_prime_state?: string | null;
  quota_window_primed_at?: string | null;
  quota_window_prime_last_error?: string | null;
};

export type AccountListStats = {
  total: number;
  active: number;
  limited: number;
  abnormal: number;
  disabled: number;
  total_quota: number;
  schedulable?: number;
  scheduling_enabled?: number;
  image_schedulable?: number;
  available_image_quota?: number;
};

export type AccountListResponse = {
  items: Account[];
  total?: number;
  offset?: number;
  limit?: number;
  stats?: AccountListStats;
};

export type AccountImportPayload = {
  access_token: string;
  accessToken?: string;
  email?: string;
  proxy?: string;
  type?: string;
  [key: string]: unknown;
};

export type AccountMutationResponse = {
  added?: number;
  skipped?: number;
  updated?: number;
  refreshed?: number;
  errors?: Array<{ error?: string }>;
  items?: Account[];
  stats?: AccountListStats;
};

export type AccountUsageRecentResponse = {
  days: number;
  dates: string[];
  by_email: Record<
    string,
    Array<{
      date: string;
      images: number;
      dialogues: number;
      images_api?: number;
      images_chat?: number;
      dialogues_real?: number;
      dialogues_nurture?: number;
    }>
  >;
};

export type AccountActivityDailyResponse = {
  days: number;
  sync_label: string;
  items: Array<{
    date: string;
    registered: number;
    uploaded: number;
    received: number;
    deleted: number;
    images?: number;
    images_api?: number;
    images_chat?: number;
    dialogues?: number;
    dialogues_real?: number;
    dialogues_nurture?: number;
  }>;
};

export type AccountRefreshAllStatus = {
  job_id?: string;
  state: string;
  running: boolean;
  started_at?: string | null;
  finished_at?: string | null;
  last_update_at?: string | null;
  total: number;
  processed: number;
  refreshed: number;
  available: number;
  became_available: number;
  quota_total?: number;
  unlimited_quota?: number;
  unknown_quota?: number;
  failed: number;
  removed?: number;
  skipped: number;
  pause_reason?: string;
  current_token?: string;
  recent?: Array<{
    index?: number;
    token?: string;
    status?: string;
    quota?: number;
    quota_unknown?: boolean;
    available?: boolean;
    error?: string;
  }>;
  options?: Record<string, unknown>;
  resource?: Record<string, unknown>;
};

export type IpNurturePreset = { id: string; label: string };
export type IpNurtureBinding = {
  binding_key: string;
  preset_id: string;
  preset_label?: string;
  weights?: number[][];
  updated_at?: string;
};

export type User = {
  id: string;
  email: string;
  role: string;
  display_name: string;
  disabled?: boolean;
  created_at?: string;
};

export type FactorPoint = { x: number; y: number };

export type JobRecord = {
  id: string;
  mode: string;
  workflow_path: string;
  ps_enabled: boolean;
  provider: string;
  input_prompt: string;
  status: string;
  error_message?: string | null;
  created_at: string;
  updated_at?: string;
};

export type JobListItem = {
  id: string;
  input_prompt: string;
  status: string;
  created_at: string;
  updated_at: string;
  result_count: number;
  thumb_url?: string | null;
};

export type JobResult = {
  id: string;
  provider: string;
  preview_url?: string | null;
  download_url?: string | null;
  thumb_url?: string | null;
  preview_b64?: string | null;
  b64_json?: string | null;
  agent_prompt?: string | null;
  revised_prompt?: string | null;
  width?: number | null;
  height?: number | null;
  size_bytes?: number | null;
};

export type JobDetail = JobRecord & { results: JobResult[] };

export const API_OFFLINE_MESSAGE = "无法连接服务器，请确认 tnexus-api 与 tnexus-worker 已启动";

export function isApiOfflineError(err: unknown): boolean {
  return err instanceof Error && err.message.includes("无法连接服务器");
}

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      ...init,
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        ...(init?.headers ?? {}),
      },
    });
  } catch {
    throw new Error(`${API_OFFLINE_MESSAGE}（${API_BASE}）`);
  }
  if (!res.ok) {
    const text = await res.text();
    let message = text || res.statusText;
    try {
      const json = JSON.parse(text) as { error?: string; message?: string };
      message = json.error ?? json.message ?? message;
    } catch {
      // keep raw text
    }
    if (res.status === 401) {
      message = "账号或密码错误";
    }
    throw new Error(message);
  }
  return res.json() as Promise<T>;
}

export const healthApi = {
  ping: async () => {
    let res: Response;
    try {
      res = await fetch(`${API_BASE}/health`, { credentials: "include" });
    } catch {
      throw new Error(`${API_OFFLINE_MESSAGE}（${API_BASE}）`);
    }
    if (!res.ok) {
      throw new Error(API_OFFLINE_MESSAGE);
    }
    return res.json() as Promise<{ status: string }>;
  },
};

export const authApi = {
  register: (body: { email: string; password: string; display_name?: string }) =>
    api<User>("/api/auth/register", { method: "POST", body: JSON.stringify(body) }),
  login: (body: { email: string; password: string }) =>
    api<User>("/api/auth/login", { method: "POST", body: JSON.stringify(body) }),
  logout: () => api<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),
  me: () => api<User>("/api/auth/me"),
  getPreferences: () => api<UserPreferences>("/api/auth/preferences"),
  savePreferences: (patch: UserPreferences) =>
    api<UserPreferences>("/api/auth/preferences", {
      method: "PATCH",
      body: JSON.stringify({ preferences: patch }),
    }),
  listUsers: () => api<User[]>("/api/auth/users"),
  setDisabled: (id: string, disabled: boolean) =>
    api<{ ok: boolean }>(`/api/auth/users/${id}/disabled`, {
      method: "POST",
      body: JSON.stringify({ disabled }),
    }),
};

export type ConversationStatePayload = ConversationState | ChatConversationState;

export const conversationsApi = {
  list: () =>
    api<
      {
        id: string;
        title: string;
        state: ConversationStatePayload;
        created_at: string;
        updated_at: string;
      }[]
    >("/api/conversations"),
  create: (body?: { title?: string; state?: ConversationStatePayload }) =>
    api<Conversation>("/api/conversations", { method: "POST", body: JSON.stringify(body ?? {}) }),
  get: (id: string) => api<Conversation>(`/api/conversations/${id}`),
  patch: (id: string, body: { title?: string; state?: ConversationStatePayload }) =>
    api<Conversation>(`/api/conversations/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
};

export const jobsApi = {
  create: (body: {
    mode: string;
    workflow_path: string;
    ps_enabled: boolean;
    provider: string;
    director_models: string[];
    director_factors: FactorPoint;
    ps_factors: FactorPoint;
    input_prompt: string;
    gen_config: GenConfig;
    conversation_id?: string | null;
    actor_image_counts: Record<string, number>;
  }) => api<{ job_id: string }>("/api/jobs", { method: "POST", body: JSON.stringify(body) }),
  list: () => api<JobRecord[]>("/api/jobs"),
  listSummaries: () => api<JobListItem[]>("/api/jobs/summaries"),
  deleteMany: (ids: string[]) =>
    api<{ ok: boolean; deleted: number }>("/api/jobs", {
      method: "DELETE",
      body: JSON.stringify({ ids }),
    }),
  get: (id: string) => api<JobDetail>(`/api/jobs/${id}`),
  getStatus: (id: string) =>
    api<{ status: string; error_message?: string | null; progress: number }>(`/api/jobs/${id}/status`),
  eventsUrl: (id: string) => `${API_BASE}/api/jobs/${id}/events`,
};

export const accountsApi = {
  list: (params?: { offset?: number; limit?: number }) => {
    const q = new URLSearchParams();
    if (params?.offset != null) q.set("offset", String(params.offset));
    if (params?.limit != null) q.set("limit", String(params.limit));
    const query = q.toString();
    return api<AccountListResponse>(`/api/accounts${query ? `?${query}` : ""}`);
  },
  reloadFromStorage: () =>
    api<{ ok: boolean; total?: number; error?: string }>("/api/accounts/reload-from-storage", {
      method: "POST",
    }),
  activityDaily: (days = 14) =>
    api<AccountActivityDailyResponse>(`/api/accounts/activity/daily?days=${days}`),
  usageRecent: (days = 6) =>
    api<AccountUsageRecentResponse>(`/api/accounts/usage/recent?days=${days}`),
  schedulableBreakdown: () =>
    api<{ buckets: Record<string, number>; total: number }>("/api/accounts/schedulable-breakdown"),
  setScheduling: (accessToken: string, enabled: boolean) =>
    api<{ ok: boolean; enabled: boolean; item?: Account; stats?: AccountListStats }>(
      "/api/accounts/scheduling",
      {
        method: "POST",
        body: JSON.stringify({ access_token: accessToken, enabled }),
      },
    ),
  schedulingBulk: (accessTokens: string[], enabled: boolean) =>
    api<{ ok: boolean; updated: number; enabled: boolean }>("/api/accounts/scheduling/bulk", {
      method: "POST",
      body: JSON.stringify({ access_tokens: accessTokens, enabled }),
    }),
  create: (tokens: string[], accounts: AccountImportPayload[] = []) =>
    api<AccountMutationResponse>("/api/accounts?include_items=false", {
      method: "POST",
      body: JSON.stringify({ tokens, accounts }),
    }),
  importBatch: (accounts: AccountImportPayload[]) =>
    api<AccountMutationResponse>("/api/accounts/import-batch?include_items=false", {
      method: "POST",
      body: JSON.stringify({ accounts }),
    }),
  exportJson: async (accessTokens: string[]) => {
    let res: Response;
    try {
      res = await fetch(`${API_BASE}/api/accounts/export`, {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ access_tokens: accessTokens, format: "json" }),
      });
    } catch {
      throw new Error(`${API_OFFLINE_MESSAGE}（${API_BASE}）`);
    }
    if (!res.ok) {
      const text = await res.text();
      let message = text || res.statusText;
      try {
        const json = JSON.parse(text) as { error?: string };
        message = json.error ?? message;
      } catch {
        // keep raw
      }
      throw new Error(message);
    }
    const blob = await res.blob();
    const disposition = res.headers.get("content-disposition") ?? "";
    const match = disposition.match(/filename="([^"]+)"/);
    const filename = match?.[1] ?? "tnexus-accounts.json";
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = filename;
    anchor.click();
    URL.revokeObjectURL(url);
  },
  bindingSlots: (params?: { week_offset?: number; timezone?: string }) => {
    const q = new URLSearchParams();
    if (params?.week_offset != null) q.set("week_offset", String(params.week_offset));
    if (params?.timezone) q.set("timezone", params.timezone);
    const query = q.toString();
    return api<BindingSlotsResponse>(`/api/accounts/usage/binding-slots${query ? `?${query}` : ""}`);
  },
  refresh: (accessTokens: string[]) =>
    api<{ progress_id: string }>("/api/accounts/refresh", {
      method: "POST",
      body: JSON.stringify({ access_tokens: accessTokens }),
    }),
  refreshProgress: (progressId: string) =>
    api<RefreshProgressResponse>(`/api/accounts/refresh/progress/${progressId}`),
  reLogin: (accessTokens: string[]) =>
    api<{ progress_id: string }>("/api/accounts/re-login", {
      method: "POST",
      body: JSON.stringify({ access_tokens: accessTokens }),
    }),
  reLoginProgress: (progressId: string) =>
    api<RefreshProgressResponse>(`/api/accounts/re-login/progress/${progressId}`),
  oauthStart: (emailHint?: string) =>
    api<OAuthLoginStartResponse>("/api/accounts/oauth/start", {
      method: "POST",
      body: JSON.stringify({ email_hint: emailHint ?? "" }),
    }),
  oauthFinish: (sessionId: string, callback: string) =>
    api<AccountMutationResponse>("/api/accounts/oauth/finish", {
      method: "POST",
      body: JSON.stringify({ session_id: sessionId, callback }),
    }),
  deleteMany: (accessTokens: string[]) =>
    api<{ removed: number; stats?: AccountListStats; items?: Account[] }>("/api/accounts?include_items=false", {
      method: "DELETE",
      body: JSON.stringify({ tokens: accessTokens }),
    }),
  update: (body: {
    access_token: string;
    type?: string;
    status?: string;
    quota?: number;
    proxy?: string;
  }) =>
    api<{ item: Account; stats?: AccountListStats }>("/api/accounts/update?include_items=false", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  softBand: (accessToken: string, percent: number | null) =>
    api<{ item: Account; stats?: AccountListStats }>("/api/accounts/soft-band?include_items=false", {
      method: "POST",
      body: JSON.stringify({
        access_token: accessToken,
        percent: percent ?? undefined,
        clear: percent == null,
      }),
    }),
  primeQuotaWindow: (body: {
    access_tokens?: string[];
    preferred_account_email?: string;
    mode?: string;
    force?: boolean;
  }) =>
    api<Record<string, unknown>>("/api/accounts/quota-window/prime", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  primeQuotaWindowStatus: () =>
    api<Record<string, unknown>>("/api/accounts/quota-window/prime/status"),
  refreshAllStart: (options: Record<string, unknown> = {}) =>
    api<AccountRefreshAllStatus>("/api/accounts/refresh-all/start", {
      method: "POST",
      body: JSON.stringify(options),
    }),
  refreshAllStatus: () => api<AccountRefreshAllStatus>("/api/accounts/refresh-all/status"),
  refreshAllStop: () =>
    api<AccountRefreshAllStatus>("/api/accounts/refresh-all/stop", { method: "POST", body: "{}" }),
  recoverOutlook: (accessToken: string) =>
    api<{ progress_id: string }>("/api/accounts/recover-outlook", {
      method: "POST",
      body: JSON.stringify({ access_token: accessToken }),
    }),
  recoverOutlookProgress: (progressId: string) =>
    api<Record<string, unknown>>(`/api/accounts/recover-outlook/progress/${encodeURIComponent(progressId)}`),
  outlookRecoveryStatus: () => api<Record<string, unknown>>("/api/accounts/outlook-recovery/status"),
  outlookRecoveryEnable: (enabled: boolean) =>
    api<Record<string, unknown>>("/api/accounts/outlook-recovery/enable", {
      method: "POST",
      body: JSON.stringify({ enabled }),
    }),
};

export type BindingSlotsResponse = {
  week_offset?: number;
  week_start?: string;
  week_end?: string;
  week_label?: string;
  weekday_labels?: string[];
  day_labels?: string[];
  timezone?: string;
  timezone_label?: string;
  by_binding?: Record<string, Record<string, number[][]>>;
};

export type RefreshProgressResponse = {
  done?: boolean;
  processed?: number;
  total?: number;
  error?: string | null;
  result?: AccountMutationResponse;
};

export type OAuthLoginStartResponse = {
  session_id: string;
  authorize_url: string;
  expires_in?: number;
};

export type ManagedImage = {
  rel: string;
  name: string;
  date: string;
  size: number;
  url: string;
  thumbnail_url?: string;
  thumb_api_url?: string;
  b64_json?: string | null;
  preview_b64?: string | null;
  created_at: string;
  duration_ms?: number;
  width?: number;
  height?: number;
  tags?: string[];
  prompt?: string;
};

export type SystemLog = {
  id: string;
  time: string;
  type: "call" | "account" | "llm_ops" | string;
  summary?: string;
  detail?: Record<string, unknown>;
};

export const logsApi = {
  list: (filters: {
    type?: string;
    start_date?: string;
    end_date?: string;
    source?: string;
    outcome?: string;
    limit?: number;
  }) => {
    const q = new URLSearchParams();
    if (filters.type) q.set("type", filters.type);
    if (filters.start_date) q.set("start_date", filters.start_date);
    if (filters.end_date) q.set("end_date", filters.end_date);
    if (filters.source) q.set("source", filters.source);
    if (filters.outcome) q.set("outcome", filters.outcome);
    if (filters.limit != null) q.set("limit", String(filters.limit));
    const query = q.toString();
    return api<{ items: SystemLog[] }>(`/api/logs${query ? `?${query}` : ""}`);
  },
  delete: (ids: string[]) =>
    api<{ removed: number }>("/api/logs/delete", {
      method: "POST",
      body: JSON.stringify({ ids }),
    }),
};

export const imagesApi = {
  list: (filters: { start_date?: string; end_date?: string }) => {
    const q = new URLSearchParams();
    if (filters.start_date) q.set("start_date", filters.start_date);
    if (filters.end_date) q.set("end_date", filters.end_date);
    const query = q.toString();
    return api<{ items: ManagedImage[] }>(`/api/images${query ? `?${query}` : ""}`);
  },
  delete: (body: { paths?: string[]; start_date?: string; end_date?: string; all_matching?: boolean }) =>
    api<{ removed: number }>("/api/images/delete", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  tags: () => api<{ tags: string[] }>("/api/images/tags"),
  setTags: (path: string, tags: string[]) =>
    api<{ ok: boolean; tags: string[] }>("/api/images/tags", {
      method: "POST",
      body: JSON.stringify({ path, tags }),
    }),
};

export type OpsSummary = {
  jobs_total: number;
  jobs_running: number;
  jobs_done: number;
  jobs_failed: number;
  results_total: number;
  accounts_total: number;
};

export const opsApi = {
  summary: () => api<OpsSummary>("/api/ops/summary"),
  nurtureStatus: () => api<Record<string, unknown>>("/api/ops/nurture/status"),
  nurtureEnqueue: (body: { prompt?: string; source?: string; access_tokens?: string[] }) =>
    api<Record<string, unknown>>("/api/ops/nurture/enqueue", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  nurtureEnable: (enabled: boolean) =>
    api<Record<string, unknown>>("/api/ops/nurture/enable", {
      method: "POST",
      body: JSON.stringify({ enabled }),
    }),
  nurtureProcessOne: (body: { prompt?: string; access_token?: string; email?: string; source?: string }) =>
    api<{ ok: boolean; chars_out?: number; latency_ms?: number }>("/api/ops/nurture/process-one", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  ipNurturePresets: () => api<{ presets: IpNurturePreset[] }>("/api/ops/ip-nurture/presets"),
  ipNurtureBindings: () =>
    api<{ bindings: Record<string, IpNurtureBinding> }>("/api/ops/ip-nurture/bindings"),
  saveIpNurtureBinding: (body: {
    binding_key: string;
    preset_id: string;
    custom_matrix?: number[][];
  }) =>
    api<IpNurtureBinding>("/api/ops/ip-nurture/bindings", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  pipelineSnapshot: () => api<Record<string, unknown>>("/api/ops/image-pipeline/snapshot"),
  riskMetrics: () => api<Record<string, unknown>>("/api/ops/risk/metrics"),
};

export const proxyApi = {
  runtime: () => api<Record<string, unknown>>("/api/proxy/runtime"),
  saveRuntime: (runtime: Record<string, unknown>) =>
    api<Record<string, unknown>>("/api/proxy/runtime", {
      method: "POST",
      body: JSON.stringify(runtime),
    }),
  test: (url: string) =>
    api<Record<string, unknown>>("/api/proxy/test", {
      method: "POST",
      body: JSON.stringify({ url }),
    }),
  webshareStatus: () => api<Record<string, unknown>>("/api/ops/webshare-cf-scan/status"),
  webshareInventory: () => api<Record<string, unknown>>("/api/ops/webshare-cf-scan/inventory"),
  webshareRunOnce: () =>
    api<Record<string, unknown>>("/api/ops/webshare-cf-scan/run-once", {
      method: "POST",
      body: "{}",
    }),
};

const GATEWAY_BASE = (process.env.NEXT_PUBLIC_GATEWAY_BASE ?? "http://localhost:8014").replace(/\/$/, "");
const GATEWAY_KEY = process.env.NEXT_PUBLIC_GATEWAY_KEY ?? "";

async function readChatStream(
  res: Response,
  onDelta: (text: string) => void,
  onImageB64?: (b64: string) => void,
) {
  const reader = res.body?.getReader();
  if (!reader) throw new Error("无响应流");
  const decoder = new TextDecoder();
  let buffer = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("data:")) continue;
      const payload = trimmed.slice(5).trim();
      if (payload === "[DONE]") return;
      try {
        const json = JSON.parse(payload) as {
          choices?: Array<{
            delta?: { content?: string; tnexus_image_b64?: string };
            message?: { tnexus_image_b64?: string };
          }>;
        };
        const choice = json.choices?.[0];
        const delta = choice?.delta;
        if (delta?.content) onDelta(delta.content);
        const imageB64 = delta?.tnexus_image_b64 ?? choice?.message?.tnexus_image_b64;
        if (imageB64 && onImageB64) onImageB64(imageB64);
      } catch {
        // skip malformed chunk
      }
    }
  }
}

export const chatApi = {
  streamCompletion: async (
    body: { model: string; messages: Array<{ role: string; content: string }>; stream?: boolean },
    onDelta: (text: string) => void,
    onImageB64?: (b64: string) => void,
  ) => {
    const stream = body.stream !== false;
    let res: Response;
    try {
      res = await fetch(`${API_BASE}/api/chat/completions`, {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ...body, stream }),
      });
    } catch {
      if (GATEWAY_BASE) {
        res = await fetch(`${GATEWAY_BASE}/v1/chat/completions`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            ...(GATEWAY_KEY ? { Authorization: `Bearer ${GATEWAY_KEY}` } : {}),
          },
          body: JSON.stringify({ ...body, stream }),
        });
      } else {
        throw new Error("无法连接对话服务");
      }
    }
    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || res.statusText);
    }
    if (stream) {
      await readChatStream(res, onDelta, onImageB64);
      return;
    }
    const data = (await res.json()) as {
      choices?: Array<{
        message?: { content?: string; tnexus_image_b64?: string };
      }>;
    };
    const message = data.choices?.[0]?.message;
    if (message?.tnexus_image_b64 && onImageB64) {
      onImageB64(message.tnexus_image_b64);
    }
    const content = message?.content ?? "";
    if (content) onDelta(content);
  },
};
