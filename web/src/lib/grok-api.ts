/**
 * Grok 独立 API client：默认经 TNexus `/api/grok/v1` 代理（cookie 鉴权，无需前端持 key）。
 * 直连：设 NEXT_PUBLIC_GROK_API_BASE=http://host:8000 并配 NEXT_PUBLIC_GROK_API_KEY。
 */

const API_BASE = process.env.NEXT_PUBLIC_API_BASE ?? "http://localhost:9000";

export const GROK_API_VIA_TNEXUS =
  process.env.NEXT_PUBLIC_GROK_API_VIA_TNEXUS !== "0" &&
  !process.env.NEXT_PUBLIC_GROK_API_BASE;

const BASE = GROK_API_VIA_TNEXUS
  ? `${API_BASE.replace(/\/$/, "")}/api/grok/v1`
  : (process.env.NEXT_PUBLIC_GROK_API_BASE ?? "/grok/v1").replace(/\/$/, "");

const KEY = process.env.NEXT_PUBLIC_GROK_API_KEY ?? "";

function authHeaders(extra?: Record<string, string>): Record<string, string> {
  const useProxy = GROK_API_VIA_TNEXUS;
  return {
    ...(extra ?? {}),
    ...(useProxy ? {} : KEY ? { Authorization: `Bearer ${KEY}` } : {}),
  };
}

function fetchOpts(init?: RequestInit): RequestInit {
  if (!GROK_API_VIA_TNEXUS) return init ?? {};
  return { ...init, credentials: "include" };
}

/** OpenAI 兼容路径：代理模式 BASE 已含 /v1 前缀。 */
function grokUrl(path: string): string {
  const p = path.startsWith("/") ? path : `/${path}`;
  if (GROK_API_VIA_TNEXUS) return `${BASE}${p}`;
  if (BASE.endsWith("/v1")) return `${BASE}${p}`;
  return `${BASE}/v1${p}`;
}

/** 嗅探 base64 图片 MIME（与 api.ts 同实现；独立拷贝避免耦合 gpt client）。 */
export function sniffImageMime(b64: string): "png" | "jpeg" | "webp" {
  const s = b64.replace(/^data:[^,]+;base64,/, "").slice(0, 16);
  if (s.startsWith("/9j/")) return "jpeg";
  if (s.startsWith("UklGR")) return "webp";
  if (s.startsWith("iVBORw0KGgo")) return "png";
  return "png";
}

async function readChatStream(
  res: Response,
  onDelta: (text: string) => void,
): Promise<void> {
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
          choices?: Array<{ delta?: { content?: string } }>;
        };
        const delta = json.choices?.[0]?.delta;
        if (delta?.content) onDelta(delta.content);
      } catch {
        // skip malformed chunk
      }
    }
  }
}

export type GrokChatContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

export type GrokChatMessage = {
  role: string;
  content: string | GrokChatContentPart[];
};

export const grokApi = {
  /** GET /v1/models → 模型 id 列表（含 grok-vision-ocr 别名）。失败返回空数组。 */
  listModels: async (): Promise<string[]> => {
    try {
      const res = await fetch(grokUrl("/models"), fetchOpts({ headers: authHeaders() }));
      if (!res.ok) return [];
      const data = (await res.json()) as { data?: Array<{ id?: string }> };
      return (data.data ?? []).map((m) => m.id).filter(Boolean) as string[];
    } catch {
      return [];
    }
  },

  /** POST /v1/chat/completions（SSE 流式；非流式也支持）。返回调度账号 ID（若有）。 */
  streamCompletion: async (
    body: {
      model: string;
      messages: GrokChatMessage[];
      stream?: boolean;
    },
    onDelta: (text: string) => void,
  ): Promise<{ accountId: number | null }> => {
    const stream = body.stream !== false;
    const res = await fetch(
      grokUrl("/chat/completions"),
      fetchOpts({
        method: "POST",
        headers: authHeaders({ "Content-Type": "application/json" }),
        body: JSON.stringify({ ...body, stream }),
      }),
    );
    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || res.statusText);
    }
    const headerId = res.headers.get("x-grok-account-id");
    let accountId = headerId ? Number.parseInt(headerId, 10) : null;
    if (accountId != null && Number.isNaN(accountId)) accountId = null;
    if (stream) {
      await readChatStream(res, onDelta);
      return { accountId };
    }
    const data = (await res.json()) as {
      choices?: Array<{ message?: { content?: string } }>;
      account_id?: number | null;
    };
    const content = data.choices?.[0]?.message?.content ?? "";
    if (content) onDelta(content);
    if (accountId == null && data.account_id != null) {
      accountId = data.account_id;
    }
    return { accountId };
  },

  /** 生图（独立端点 /v1/images/generations）。返回 b64 数组。 */
  generateImage: async (
    prompt: string,
    n: number,
    opts?: { size?: string; aspectRatio?: string },
  ): Promise<string[]> => {
    const size = opts?.aspectRatio ?? opts?.size ?? "1:1";
    const res = await fetch(
      grokUrl("/images/generations"),
      fetchOpts({
        method: "POST",
        headers: authHeaders({ "Content-Type": "application/json" }),
        body: JSON.stringify({ prompt, n, size, response_format: "b64_json" }),
      }),
    );
    if (!res.ok) {
      const text = await res.text();
      let message = text || res.statusText;
      try {
        const json = JSON.parse(text) as {
          error?: { message?: string } | string;
          message?: string;
        };
        message =
          typeof json.error === "string"
            ? json.error
            : (json.error?.message ?? json.message ?? message);
      } catch {
        // keep raw text
      }
      if (res.status === 500 || res.status === 503) {
        message = `${message}（需 gateway 开启 GROK_IMAGE_ENABLED=1）`;
      }
      throw new Error(message);
    }
    const data = (await res.json()) as {
      data?: Array<{ b64_json?: string; url?: string }>;
    };
    return (data.data ?? []).map((d) => d.b64_json ?? d.url ?? "").filter(Boolean);
  },

  /** OCR：走 /v1/chat/completions 带图附件（grok-vision-ocr 别名），返回识别文本。 */
  extractText: async (dataUrl: string, prompt?: string): Promise<string> => {
    const body = {
      model: "grok-vision-ocr",
      stream: false,
      messages: [
        {
          role: "user",
          content: [
            {
              type: "text",
              text: prompt ?? "请提取这张图片中的全部文字内容，按原有顺序输出；只输出识别到的文字。",
            },
            { type: "image_url", image_url: { url: dataUrl } },
          ],
        },
      ],
    };
    const res = await fetch(
      grokUrl("/chat/completions"),
      fetchOpts({
        method: "POST",
        headers: authHeaders({ "Content-Type": "application/json" }),
        body: JSON.stringify(body),
      }),
    );
    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || res.statusText);
    }
    const data = (await res.json()) as {
      choices?: Array<{ message?: { content?: string } }>;
    };
    const content = data.choices?.[0]?.message?.content ?? "";
    if (!content.trim()) {
      throw new Error("未识别到文字（空响应）");
    }
    return content;
  },
};
