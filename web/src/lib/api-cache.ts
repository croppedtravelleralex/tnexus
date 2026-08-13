type CacheEntry<T> = { data: T; at: number };

const store = new Map<string, CacheEntry<unknown>>();

export function getCached<T>(key: string, maxAgeMs: number): T | null {
  const hit = store.get(key);
  if (!hit) return null;
  if (Date.now() - hit.at > maxAgeMs) return null;
  return hit.data as T;
}

export function setCached<T>(key: string, data: T) {
  store.set(key, { data, at: Date.now() });
}

export function invalidateCache(prefix?: string) {
  if (!prefix) {
    store.clear();
    return;
  }
  for (const key of store.keys()) {
    if (key.startsWith(prefix)) store.delete(key);
  }
}

export function deleteCached(key: string) {
  store.delete(key);
}

export async function fetchWithCache<T>(
  key: string,
  fetcher: () => Promise<T>,
  maxAgeMs = 30_000,
  options?: { force?: boolean },
): Promise<{ data: T; fromCache: boolean }> {
  if (!options?.force) {
    const hit = getCached<T>(key, maxAgeMs);
    if (hit !== null) return { data: hit, fromCache: true };
  }
  const data = await fetcher();
  setCached(key, data);
  return { data, fromCache: false };
}
