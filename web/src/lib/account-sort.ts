import type { Account } from "@/lib/api";
import { displayAccountType, bindingLabelForAccount } from "@/lib/account-display";

export type SortKey = { key: string; dir: "asc" | "desc" };

function schedulingRank(account: Account): number {
  const receive = String(account.panda_receive_state ?? "").trim().toLowerCase();
  if (!receive) return 2;
  if (receive === "verified_ready" || receive === "verified" || receive === "local_verified") return 2;
  return 0;
}

function recordTotal(account: Account): number {
  return Number(account.success ?? 0) + Number(account.fail ?? 0);
}

function windowRank(account: Account): string {
  return String(account.restore_at || account.quota_window_primed_at || account.quota_window_prime_state || "");
}

export function compareAccounts(a: Account, b: Account, key: string, dir: "asc" | "desc"): number {
  const mul = dir === "asc" ? 1 : -1;
  let av: string | number = "";
  let bv: string | number = "";

  switch (key) {
    case "email":
      av = String(a.email || a.access_token || "").toLowerCase();
      bv = String(b.email || b.access_token || "").toLowerCase();
      break;
    case "type":
      av = displayAccountType(a);
      bv = displayAccountType(b);
      break;
    case "status":
      av = String(a.status || "");
      bv = String(b.status || "");
      break;
    case "scheduling":
      av = schedulingRank(a);
      bv = schedulingRank(b);
      break;
    case "record":
      av = recordTotal(a);
      bv = recordTotal(b);
      break;
    case "proxy":
      av = bindingLabelForAccount(a).toLowerCase();
      bv = bindingLabelForAccount(b).toLowerCase();
      break;
    case "quota":
      av = Number(a.quota || 0);
      bv = Number(b.quota || 0);
      break;
    case "window":
      av = windowRank(a);
      bv = windowRank(b);
      break;
    case "inflight":
      av = Number(a.image_inflight ?? 0);
      bv = Number(b.image_inflight ?? 0);
      break;
    case "created_at":
    default:
      av = String(a.created_at || "");
      bv = String(b.created_at || "");
      break;
  }

  if (typeof av === "number" && typeof bv === "number") {
    if (av < bv) return -1 * mul;
    if (av > bv) return 1 * mul;
    return 0;
  }
  const as = String(av);
  const bs = String(bv);
  if (as < bs) return -1 * mul;
  if (as > bs) return 1 * mul;
  return 0;
}

export function sortAccounts(items: Account[], sortKeys: SortKey[]): Account[] {
  const keys = sortKeys.length ? sortKeys : [{ key: "created_at", dir: "desc" as const }];
  return [...items].sort((a, b) => {
    for (const { key, dir } of keys) {
      const r = compareAccounts(a, b, key, dir);
      if (r !== 0) return r;
    }
    return 0;
  });
}
