/**
 * 测试 api-cache 内存缓存的清除逻辑（含登出和用户身份切换场景）。
 * 运行：node --test web/scripts/test_cache_clear.mjs（从仓库根执行）
 *
 * 此文件内联纯 JS 版本的缓存核心逻辑（镜像 src/lib/api-cache.ts），
 * 不依赖 Next.js / React / TypeScript 运行时。
 */

import { test } from "node:test";
import assert from "node:assert/strict";

// ── 内联核心缓存逻辑（对应 api-cache.ts）──────────────────────────────────

const store = new Map();

function getCached(key, maxAgeMs) {
  const hit = store.get(key);
  if (!hit) return null;
  if (Date.now() - hit.at > maxAgeMs) return null;
  return hit.data;
}

function setCached(key, data) {
  store.set(key, { data, at: Date.now() });
}

function invalidateCache(prefix) {
  if (!prefix) { store.clear(); return; }
  for (const key of store.keys()) {
    if (key.startsWith(prefix)) store.delete(key);
  }
}

function clearAllCaches() {
  invalidateCache();
}

// ── 内联身份切换检测逻辑（对应 auth.tsx prevUserIdRef 模式）──────────────

function makeIdentityTracker() {
  let prevId = undefined; // undefined = 首次加载前
  return {
    onUserLoad(userId) {
      const changed = prevId !== undefined && prevId !== userId;
      if (changed) clearAllCaches();
      prevId = userId;
      return changed;
    },
    onLogout() {
      clearAllCaches();
      prevId = null;
    },
    prevId() { return prevId; },
  };
}

// ── 测试用例 ─────────────────────────────────────────────────────────────

test("setCached 写入后 getCached 在 TTL 内可读取", () => {
  store.clear();
  setCached("test:foo", { value: 42 });
  assert.deepEqual(getCached("test:foo", 30_000), { value: 42 });
});

test("getCached 在 TTL 过期后返回 null", () => {
  store.clear();
  store.set("stale:1", { data: "old", at: Date.now() - 60_000 });
  assert.equal(getCached("stale:1", 30_000), null);
});

test("clearAllCaches 清除所有条目（images 和 job 前缀均被清除）", () => {
  store.clear();
  setCached("images:2026-01-01:2026-01-31", [1, 2, 3]);
  setCached("job:abc123", { id: "abc123" });
  setCached("job:def456", { id: "def456" });

  assert.notEqual(getCached("images:2026-01-01:2026-01-31", 30_000), null);
  assert.notEqual(getCached("job:abc123", 300_000), null);

  clearAllCaches();

  assert.equal(getCached("images:2026-01-01:2026-01-31", 30_000), null, "images 缓存应已清除");
  assert.equal(getCached("job:abc123", 300_000), null, "job 缓存应已清除");
  assert.equal(getCached("job:def456", 300_000), null, "job 缓存（第二条）应已清除");
  assert.equal(store.size, 0, "store 应为空");
});

test("invalidateCache 按前缀仅清除匹配条目", () => {
  store.clear();
  setCached("images:2026-01-01:2026-01-31", ["img1"]);
  setCached("job:abc123", { id: "abc123" });

  invalidateCache("images:");

  assert.equal(getCached("images:2026-01-01:2026-01-31", 30_000), null, "images 缓存应已清除");
  assert.notEqual(getCached("job:abc123", 300_000), null, "job 缓存不应受影响");
});

test("身份切换：首次加载不触发 clearAllCaches", () => {
  store.clear();
  const tracker = makeIdentityTracker();
  setCached("images:today", ["user-a-data"]);

  const changed = tracker.onUserLoad("user-a");
  assert.equal(changed, false, "首次加载不是身份切换");
  assert.notEqual(getCached("images:today", 30_000), null, "首次加载不应清除缓存");
});

test("身份切换：同一用户重复 onUserLoad 不清除缓存", () => {
  store.clear();
  const tracker = makeIdentityTracker();
  setCached("images:today", ["user-a-data"]);

  tracker.onUserLoad("user-a");
  const changed = tracker.onUserLoad("user-a");
  assert.equal(changed, false, "同一用户再次加载不是身份切换");
  assert.notEqual(getCached("images:today", 30_000), null, "缓存不应被清除");
});

test("身份切换：用户 A → 用户 B 触发 clearAllCaches", () => {
  store.clear();
  const tracker = makeIdentityTracker();

  setCached("images:today", ["user-a-data"]);
  tracker.onUserLoad("user-a");

  // 同一浏览器中切换到用户 B
  const changed = tracker.onUserLoad("user-b");
  assert.equal(changed, true, "检测到身份变化");
  assert.equal(getCached("images:today", 30_000), null, "用户 A 的缓存应在身份切换后被清除");
});

test("登出：onLogout 立即清除缓存并重置 prevId", () => {
  store.clear();
  const tracker = makeIdentityTracker();

  tracker.onUserLoad("user-a");
  setCached("images:today", ["user-a-data"]);

  tracker.onLogout();
  assert.equal(getCached("images:today", 30_000), null, "登出后缓存应被清除");
  assert.equal(tracker.prevId(), null, "登出后 prevId 应为 null");
});

test("登出后重新登入：prevId 为 null，非 undefined，首次加载不视为身份切换", () => {
  store.clear();
  const tracker = makeIdentityTracker();

  tracker.onUserLoad("user-a");
  tracker.onLogout(); // prevId = null

  // 用户 A 重新登入
  setCached("images:today", ["new-data"]);
  // prevId(null) !== "user-a"，视为身份切换（null → user-a），会清除缓存
  const changed = tracker.onUserLoad("user-a");
  assert.equal(changed, true, "登出后重新登入视为身份切换");
  assert.equal(getCached("images:today", 30_000), null, "重新登入应清除登出前积累的缓存");
});
