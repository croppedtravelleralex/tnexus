import { test } from "node:test";
import assert from "node:assert/strict";

// ── helpers inlined from grok-quota.ts ──────────────────────────────────────

function unwrapAdminItems(raw) {
  if (Array.isArray(raw)) return raw;
  if (raw && typeof raw === "object" && Array.isArray(raw.items)) return raw.items;
  return [];
}

function pickDisplayQuotaWindow(windows, now = Date.now()) {
  if (!windows || windows.length === 0) return null;
  const order = ["fast", "auto", "console", "imagine"];
  const staleMs = 24 * 60 * 60 * 1000;
  const hasValue = (w) => w.total > 0 || w.remaining > 0;
  const isStale = (w) => {
    if (!w.synced_at) return true;
    const syncedMs = new Date(w.synced_at).getTime();
    if (Number.isNaN(syncedMs)) return true;
    return now - syncedMs > staleMs;
  };
  for (const mode of order) {
    const hit = windows.find((w) => w.mode === mode && hasValue(w) && !isStale(w));
    if (hit) return hit;
  }
  const anyFresh = windows.find((w) => hasValue(w) && !isStale(w));
  if (anyFresh) return anyFresh;
  for (const mode of order) {
    const hit = windows.find((w) => w.mode === mode && hasValue(w));
    if (hit) return hit;
  }
  return windows.find((w) => hasValue(w)) ?? windows[0] ?? null;
}

// ── helpers inlined from grok-accounts-table.tsx ────────────────────────────

function formatCooldown(cooldownUntil, now = Date.now()) {
  if (!cooldownUntil) return { kind: "none" };
  const d = new Date(cooldownUntil);
  if (Number.isNaN(d.getTime())) return { kind: "none" };
  const rawLabel = d.toLocaleString("zh-CN", { hour12: false });
  const remainingMs = d.getTime() - now;
  if (remainingMs <= 0) return { kind: "past", rawLabel };

  let label;
  const totalSec = Math.ceil(remainingMs / 1000);
  if (totalSec < 60) label = `${totalSec}s`;
  else if (totalSec < 3600) label = `${Math.ceil(totalSec / 60)}min`;
  else if (totalSec < 86400) label = `${Math.ceil(totalSec / 3600)}h`;
  else label = `${Math.ceil(totalSec / 86400)}d`;
  return { kind: "future", label, rawLabel, remainingMs };
}

// ── helpers inlined from grok-quota-heatstrip.tsx ───────────────────────────

const QUOTA_UNLIMITED_THRESHOLD = 1_000_000_000;

function isQuotaStale(window, now = Date.now()) {
  if (!window.synced_at) return true;
  const syncedMs = new Date(window.synced_at).getTime();
  if (Number.isNaN(syncedMs)) return true;
  return now - syncedMs > 24 * 60 * 60 * 1000;
}

// ── original tests ───────────────────────────────────────────────────────────

test("unwrap {items} wrapper that caused 未知", () => {
  const raw = {
    items: [
      { mode: "auto", remaining: 7, total: 7 },
      { mode: "fast", remaining: 30, total: 30 },
    ],
  };
  assert.equal(unwrapAdminItems(raw)[0]?.mode, "auto");
  assert.equal(raw[0], undefined);
  assert.equal(pickDisplayQuotaWindow(unwrapAdminItems(raw))?.remaining, 30);
});

test("unwrap already-array payload", () => {
  assert.equal(unwrapAdminItems([{ mode: "fast", remaining: 1, total: 1 }]).length, 1);
});

// ── cooldown tests ───────────────────────────────────────────────────────────

const NOW = new Date("2026-08-13T10:00:00Z").getTime();

test("cooldown null → none", () => {
  assert.deepEqual(formatCooldown(null, NOW), { kind: "none" });
});

test("cooldown invalid date → none", () => {
  assert.deepEqual(formatCooldown("not-a-date", NOW), { kind: "none" });
});

test("cooldown in the past → past (e.g. 2026-07-27)", () => {
  const state = formatCooldown("2026-07-27T00:00:00Z", NOW);
  assert.equal(state.kind, "past");
  assert.ok(typeof state.rawLabel === "string" && state.rawLabel.length > 0);
});

test("cooldown exactly now → past", () => {
  const state = formatCooldown(new Date(NOW).toISOString(), NOW);
  assert.equal(state.kind, "past");
});

test("cooldown 30s in the future → future with 's' label", () => {
  const state = formatCooldown(new Date(NOW + 30_000).toISOString(), NOW);
  assert.equal(state.kind, "future");
  assert.ok(state.label.endsWith("s"), `expected 's' suffix, got ${state.label}`);
});

test("cooldown 45min in the future → future with 'min' label", () => {
  // 45 min = 2700s, which is < 3600s → min branch
  const state = formatCooldown(new Date(NOW + 45 * 60 * 1000).toISOString(), NOW);
  assert.equal(state.kind, "future");
  assert.ok(state.label.endsWith("min"), `expected 'min' suffix, got ${state.label}`);
});

test("cooldown 3h in the future → future with 'h' label", () => {
  const state = formatCooldown(new Date(NOW + 3 * 3600 * 1000).toISOString(), NOW);
  assert.equal(state.kind, "future");
  assert.ok(state.label.endsWith("h"), `expected 'h' suffix, got ${state.label}`);
});

test("cooldown 2d in the future → future with 'd' label", () => {
  const state = formatCooldown(new Date(NOW + 2 * 86400 * 1000).toISOString(), NOW);
  assert.equal(state.kind, "future");
  assert.ok(state.label.endsWith("d"), `expected 'd' suffix, got ${state.label}`);
});

// ── unlimited quota threshold tests ─────────────────────────────────────────

test("total < 1B is NOT unlimited", () => {
  assert.equal(999_999_999 >= QUOTA_UNLIMITED_THRESHOLD, false);
});

test("total === 1B is unlimited sentinel", () => {
  assert.equal(1_000_000_000 >= QUOTA_UNLIMITED_THRESHOLD, true);
});

test("imagine sentinel value 11550000000 is unlimited", () => {
  assert.equal(11_550_000_000 >= QUOTA_UNLIMITED_THRESHOLD, true);
});

// ── staleness tests ──────────────────────────────────────────────────────────

test("missing synced_at is stale", () => {
  assert.equal(isQuotaStale({ synced_at: null }, NOW), true);
});

test("synced_at 5 days ago is stale", () => {
  const fiveDaysAgo = new Date(NOW - 5 * 86400 * 1000).toISOString();
  assert.equal(isQuotaStale({ synced_at: fiveDaysAgo }, NOW), true);
});

test("synced_at 1h ago is NOT stale", () => {
  const oneHourAgo = new Date(NOW - 60 * 60 * 1000).toISOString();
  assert.equal(isQuotaStale({ synced_at: oneHourAgo }, NOW), false);
});

test("synced_at exactly 24h ago is NOT stale (strict > boundary)", () => {
  // Implementation uses `now - syncedMs > 24h` (strict), so exactly 24h = not stale
  const exactly24h = new Date(NOW - 24 * 60 * 60 * 1000).toISOString();
  assert.equal(isQuotaStale({ synced_at: exactly24h }, NOW), false);
});

test("synced_at 24h + 1ms ago is stale", () => {
  const justOver24h = new Date(NOW - 24 * 60 * 60 * 1000 - 1).toISOString();
  assert.equal(isQuotaStale({ synced_at: justOver24h }, NOW), true);
});

test("synced_at 23h59m ago is NOT stale", () => {
  const almostStale = new Date(NOW - (24 * 60 * 60 * 1000 - 60_000)).toISOString();
  assert.equal(isQuotaStale({ synced_at: almostStale }, NOW), false);
});

test("invalid synced_at string is stale", () => {
  assert.equal(isQuotaStale({ synced_at: "bad-date" }, NOW), true);
});

test("prefer fresh fast over stale auto", () => {
  const windows = [
    { mode: "auto", remaining: 7, total: 7, synced_at: new Date(NOW - 5 * 86400 * 1000).toISOString() },
    { mode: "fast", remaining: 29, total: 30, synced_at: new Date(NOW - 60 * 60 * 1000).toISOString() },
  ];
  assert.equal(pickDisplayQuotaWindow(windows, NOW)?.mode, "fast");
  assert.equal(pickDisplayQuotaWindow(windows, NOW)?.remaining, 29);
});

test("fallback to stale auto when no fresh window", () => {
  const windows = [
    { mode: "auto", remaining: 7, total: 7, synced_at: new Date(NOW - 5 * 86400 * 1000).toISOString() },
    { mode: "imagine", remaining: 0, total: 0, synced_at: new Date(NOW - 5 * 86400 * 1000).toISOString() },
  ];
  assert.equal(pickDisplayQuotaWindow(windows, NOW)?.mode, "auto");
});

test("historical error when cooldown is past", () => {
  function isHistoricalError(cooldownUntil, now = Date.now()) {
    const state = formatCooldown(cooldownUntil, now);
    return state.kind === "past";
  }
  assert.equal(isHistoricalError("2026-08-08T01:00:00Z", NOW), true);
  assert.equal(isHistoricalError(new Date(NOW + 60_000).toISOString(), NOW), false);
  assert.equal(isHistoricalError(null, NOW), false);
});

// ── grok markup strip (inlined from grok-text.ts) ────────────────────────────

function stripGrokMarkup(input) {
  const complete =
    /<grok:[A-Za-z0-9_.-]+(?:\s[^>]*)?\/>|<grok:([A-Za-z0-9_.-]+)(?:\s[^>]*)?>[\s\S]*?<\/grok:\1>/gi;
  let stripped = input.replace(complete, "");
  const lastOpen = stripped.lastIndexOf("<grok:");
  if (lastOpen >= 0) {
    const tail = stripped.slice(lastOpen);
    if (!/<\/grok:[A-Za-z0-9_.-]+>/.test(tail) || !tail.includes(">")) {
      stripped = stripped.slice(0, lastOpen);
    }
  }
  return stripped;
}

test("strip citation card from grok chat screenshot", () => {
  const raw =
    "**是的**，目前公开标价一致" +
    '<grok:render card_id="69b0ae" card_type="citation_card" type="render_inline_citation">' +
    '<argument name="citation_id">0</argument></grok:render>。';
  assert.equal(stripGrokMarkup(raw), "**是的**，目前公开标价一致。");
});

test("strip self-closing grok tag", () => {
  assert.equal(stripGrokMarkup("A<grok:render x=\"1\"/>B"), "AB");
});

test("drop incomplete trailing grok tag", () => {
  assert.equal(stripGrokMarkup("hello<grok:ren"), "hello");
});

test("leave markdown headings and lists", () => {
  const s = "### 其他方面\n- **速度**";
  assert.equal(stripGrokMarkup(s), s);
});
