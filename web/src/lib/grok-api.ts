/**
 * Grok 独立 API client：直连 grok 子系统（:8000 /v1/*），与 gpt 一侧（:8014）完全隔离。
 *
 * - BASE 缺省 `/grok/v1`（同源相对路径，生产由 nginx 反代 /grok/v1/ → 127.0.0.1:8000）；
 *   本地开发可用 `NEXT_PUBLIC_GROK_API_BASE=http://localhost:8000` 覆盖。
 * - KEY = `NEXT_PUBLIC_GROK_API_KEY`（即 GROK_GATEWAY_AUTH_KEY 的公开转发值），缺省空。
 */

const BASE = (process.env.NEXT_PUBLIC_GROK_API_BASE ?? "/grok/v1").replace(/\/$/, "");
const KEY = process.env.NEXT_PUBLIC_GROK_API_KEY ?? "";

function authHeaders(extra?: Record<string, string>): Record<string, string> {
  return {
    ...(extra ?? {}),
    ...(KEY ? { Authorization: `Bearer ${KEY}` } : {}),
  };
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

export const grokApi = {
  /** GET /v1/models → 模型 id 列表（含 grok-vision-ocr 别名）。失败返回空数组。 */
  listModels: async (): Promise<string[]> => {
    try {
      const res = await fetch(`${BASE}/v1/models`, { headers: authHeaders() });
      if (!res.ok) return [];
      const data = (await res.json()) as { data?: Array<{ id?: string }> };
      return (data.data ?? []).map((m) => m.id).filter(Boolean) as string[];
    } catch {
      return [];
    }
  },

  /** POST /v1/chat/completions（SSE 流式；非流式也支持）。只走 grok 网关。 */
  streamCompletion: async (
    body: {
      model: string;
      messages: Array<{ role: string; content: string }>;
      stream?: boolean;
    },
    onDelta: (text: string) => void,
  ): Promise<void> => {
    const stream = body.stream !== false;
    const res = await fetch(`${BASE}/v1/chat/completions`, {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({ ...body, stream }),
    });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(text || res.statusText);
    }
    if (stream) {
      await readChatStream(res, onDelta);
      return;
    }
    const data = (await res.json()) as {
      choices?: Array<{ message?: { content?: string } }>;
    };
    const content = data.choices?.[0]?.message?.content ?? "";
    if (content) onDelta(content);
  },

  /** 生图（独立端点 /v1/images/generations）。返回 b64 数组。 */
  generateImage: async (prompt: string, n: number): Promise<string[]> => {
    const res = await fetch(`${BASE}/v1/images/generations`, {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({ prompt, n, response_format: "b64_json" }),
    });
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
    const res = await fetch(`${BASE}/v1/chat/completions`, {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify(body),
    });
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
