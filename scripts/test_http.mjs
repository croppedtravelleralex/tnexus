import { test } from "node:test";
import assert from "node:assert/strict";

// ── helpers inlined from http.ts ─────────────────────────────────────────────

function unwrapItems(raw) {
  if (Array.isArray(raw)) return raw;
  if (raw && typeof raw === "object" && Array.isArray(raw.items)) return raw.items;
  return [];
}

function extractErrorMessage(text, statusText) {
  const fallback = text || statusText;
  try {
    const json = JSON.parse(text);
    if (json.error) {
      if (typeof json.error === "string") return json.error;
      if (typeof json.error === "object" && json.error.message) return json.error.message;
    }
    if (json.message) return json.message;
  } catch {
    // 保留原始文本
  }
  return fallback;
}

// ── unwrapItems ───────────────────────────────────────────────────────────────

test("unwrapItems: bare array → identity", () => {
  const arr = [{ mode: "fast" }, { mode: "auto" }];
  assert.deepEqual(unwrapItems(arr), arr);
});

test("unwrapItems: { items: [...] } → unwrapped array", () => {
  const raw = { items: [{ mode: "fast" }, { mode: "auto" }] };
  assert.deepEqual(unwrapItems(raw), raw.items);
});

test("unwrapItems: null → []", () => {
  assert.deepEqual(unwrapItems(null), []);
});

test("unwrapItems: undefined → []", () => {
  assert.deepEqual(unwrapItems(undefined), []);
});

test("unwrapItems: {} (no items field) → []", () => {
  assert.deepEqual(unwrapItems({}), []);
});

test("unwrapItems: { items: null } (non-array items) → []", () => {
  assert.deepEqual(unwrapItems({ items: null }), []);
});

test("unwrapItems: { items: [] } → empty array", () => {
  assert.deepEqual(unwrapItems({ items: [] }), []);
});

test("unwrapItems: reproduces the 「未知」 quota bug — {items:[...]} not bare array", () => {
  const serverResponse = {
    items: [
      { mode: "auto", remaining: 7, total: 7 },
      { mode: "fast", remaining: 30, total: 30 },
    ],
  };
  // 旧客户端 value[0] 会是 undefined；unwrapItems 后可正常取
  const items = unwrapItems(serverResponse);
  assert.equal(items[0]?.mode, "auto");
  assert.equal(serverResponse[0], undefined, "直接下标访问仍为 undefined");
});

// ── extractErrorMessage ───────────────────────────────────────────────────────

test("extractErrorMessage: { error: 'msg' } → error field wins over message", () => {
  const text = JSON.stringify({ error: "forbidden", message: "ignored" });
  assert.equal(extractErrorMessage(text, "Status Text"), "forbidden");
});

test("extractErrorMessage: { error: { message: 'x' } } → nested message", () => {
  const text = JSON.stringify({ error: { message: "nested error detail" }, message: "also ignored" });
  assert.equal(extractErrorMessage(text, "Status Text"), "nested error detail");
});

test("extractErrorMessage: { message: 'msg' } → top-level message (no error field)", () => {
  const text = JSON.stringify({ message: "only message here" });
  assert.equal(extractErrorMessage(text, "Status Text"), "only message here");
});

test("extractErrorMessage: malformed JSON → raw body text", () => {
  assert.equal(extractErrorMessage("not json at all", "500 error"), "not json at all");
});

test("extractErrorMessage: empty body → statusText fallback", () => {
  assert.equal(extractErrorMessage("", "Internal Server Error"), "Internal Server Error");
});

test("extractErrorMessage: both body and statusText empty → empty string", () => {
  assert.equal(extractErrorMessage("", ""), "");
});

test("extractErrorMessage: empty error string → falls through to message field", () => {
  const text = JSON.stringify({ error: "", message: "fallback from message" });
  assert.equal(extractErrorMessage(text, "Status"), "fallback from message");
});

test("extractErrorMessage: error object without message → falls through to message field", () => {
  const text = JSON.stringify({ error: {}, message: "body message" });
  assert.equal(extractErrorMessage(text, "Status"), "body message");
});

test("extractErrorMessage: plain text body (no JSON) → raw text", () => {
  assert.equal(extractErrorMessage("Unauthorized", "401"), "Unauthorized");
});
