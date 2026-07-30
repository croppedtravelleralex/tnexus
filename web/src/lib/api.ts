const API_BASE = process.env.NEXT_PUBLIC_API_BASE ?? "http://localhost:9000";

import type { Conversation, ConversationState } from "@/lib/conversations";
import type { GenConfig } from "@/lib/gen-config";
import type { UserPreferences } from "@/lib/studio-layout";

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
  agent_prompt?: string | null;
  revised_prompt?: string | null;
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
  savePreferences: async (patch: UserPreferences) => {
    const current = await authApi.getPreferences();
    return api<UserPreferences>("/api/auth/preferences", {
      method: "PATCH",
      body: JSON.stringify({ preferences: { ...current, ...patch } }),
    });
  },
  listUsers: () => api<User[]>("/api/auth/users"),
  setDisabled: (id: string, disabled: boolean) =>
    api<{ ok: boolean }>(`/api/auth/users/${id}/disabled`, {
      method: "POST",
      body: JSON.stringify({ disabled }),
    }),
};

export const conversationsApi = {
  list: () =>
    api<
      {
        id: string;
        title: string;
        state: ConversationState;
        created_at: string;
        updated_at: string;
      }[]
    >("/api/conversations"),
  create: (body?: { title?: string; state?: ConversationState }) =>
    api<Conversation>("/api/conversations", { method: "POST", body: JSON.stringify(body ?? {}) }),
  get: (id: string) => api<Conversation>(`/api/conversations/${id}`),
  patch: (id: string, body: { title?: string; state?: ConversationState }) =>
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
  eventsUrl: (id: string) => `${API_BASE}/api/jobs/${id}/events`,
};
