/**
 * 端侧生图缓存（File System Access API + IndexedDB 元数据）。
 * 缩略图/预览/下载均优先读本地缓存；缓存缺失则报错（不回落远程）。
 */

type DirHandleWithPerm = FileSystemDirectoryHandle & {
  queryPermission?: (desc: { mode: "read" | "readwrite" }) => Promise<PermissionState>;
  requestPermission?: (desc: { mode: "read" | "readwrite" }) => Promise<PermissionState>;
};

type WindowWithDirPicker = Window & {
  showDirectoryPicker?: (opts: { mode: "read" | "readwrite" }) => Promise<FileSystemDirectoryHandle>;
};

const DB_NAME = "tnexus-image-cache";
const STORE = "meta";
const HANDLE_KEY = "dir-handle";
const CONFIG_KEY = "cache-config";

export type ClientCacheConfig = {
  /** 用户可见目录名（仅展示） */
  directoryName: string;
  /** 是否已授权 */
  ready: boolean;
};

type CacheMeta = {
  resultId: string;
  jobId: string;
  fileName: string;
  mime: string;
  sizeBytes?: number;
  savedAt: string;
};

function supportsDirectoryPicker(): boolean {
  return typeof window !== "undefined" && "showDirectoryPicker" in window;
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function idbGet<T>(key: string): Promise<T | undefined> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readonly");
    const req = tx.objectStore(STORE).get(key);
    req.onsuccess = () => resolve(req.result as T | undefined);
    req.onerror = () => reject(req.error);
  });
}

async function idbSet(key: string, value: unknown): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

async function getDirHandle(): Promise<FileSystemDirectoryHandle | null> {
  const handle = await idbGet<FileSystemDirectoryHandle>(HANDLE_KEY);
  return handle ?? null;
}

export async function getClientCacheConfig(): Promise<ClientCacheConfig> {
  const cfg = await idbGet<ClientCacheConfig>(CONFIG_KEY);
  return cfg ?? { directoryName: "", ready: false };
}

export async function isClientCacheReady(): Promise<boolean> {
  const cfg = await getClientCacheConfig();
  if (!cfg.ready) return false;
  const handle = await getDirHandle();
  if (!handle) return false;
  try {
    const h = handle as DirHandleWithPerm;
    if (!h.queryPermission) return true;
    const perm = await h.queryPermission({ mode: "readwrite" });
    return perm === "granted";
  } catch {
    return false;
  }
}

/** 选择/重新授权缓存目录。 */
export async function pickClientCacheDirectory(): Promise<ClientCacheConfig> {
  if (!supportsDirectoryPicker()) {
    throw new Error("当前浏览器不支持目录选择（需 Chrome/Edge 桌面版）");
  }
  const handle = await (window as WindowWithDirPicker).showDirectoryPicker!({ mode: "readwrite" });
  await idbSet(HANDLE_KEY, handle);
  const cfg: ClientCacheConfig = { directoryName: handle.name, ready: true };
  await idbSet(CONFIG_KEY, cfg);
  return cfg;
}

/** 确保目录读写权限（切换会话后可能需重新授权）。 */
export async function ensureClientCachePermission(): Promise<FileSystemDirectoryHandle> {
  const handle = await getDirHandle();
  if (!handle) {
    throw new Error("未设置端侧缓存目录，请先在「设置」中选择");
  }
  const h = handle as DirHandleWithPerm;
  if (!h.queryPermission || !h.requestPermission) return handle;
  const perm = await h.queryPermission({ mode: "readwrite" });
  if (perm !== "granted") {
    const req = await h.requestPermission({ mode: "readwrite" });
    if (req !== "granted") {
      throw new Error("端侧缓存目录未授权，请重新选择目录");
    }
  }
  return handle;
}

function cacheFileName(resultId: string, ext: string) {
  return `${resultId}.${ext}`;
}

function extFromMime(mime: string): string {
  if (mime.includes("jpeg") || mime.includes("jpg")) return "jpg";
  if (mime.includes("webp")) return "webp";
  return "png";
}

/** 将远程图片写入端侧缓存。 */
export async function saveImageToClientCache(opts: {
  jobId: string;
  resultId: string;
  downloadUrl: string;
  mime?: string;
}): Promise<CacheMeta> {
  const dir = await ensureClientCachePermission();
  const res = await fetch(opts.downloadUrl, { credentials: "include" });
  if (!res.ok) {
    throw new Error(`下载失败 HTTP ${res.status}`);
  }
  const blob = await res.blob();
  const mime = opts.mime || blob.type || "image/png";
  const fileName = cacheFileName(opts.resultId, extFromMime(mime));
  const fileHandle = await dir.getFileHandle(fileName, { create: true });
  const writable = await fileHandle.createWritable();
  await writable.write(blob);
  await writable.close();
  const meta: CacheMeta = {
    resultId: opts.resultId,
    jobId: opts.jobId,
    fileName,
    mime,
    sizeBytes: blob.size,
    savedAt: new Date().toISOString(),
  };
  await idbSet(`meta:${opts.resultId}`, meta);
  return meta;
}

const blobUrlCache = new Map<string, string>();

/** 从端侧缓存读取 blob URL；缺失抛错。 */
export async function getClientCachedBlobUrl(resultId: string): Promise<string> {
  const cached = blobUrlCache.get(resultId);
  if (cached) return cached;
  const dir = await ensureClientCachePermission();
  const meta = await idbGet<CacheMeta>(`meta:${resultId}`);
  const fileName = meta?.fileName ?? cacheFileName(resultId, "png");
  try {
    const fileHandle = await dir.getFileHandle(fileName);
    const file = await fileHandle.getFile();
    const url = URL.createObjectURL(file);
    blobUrlCache.set(resultId, url);
    return url;
  } catch {
    throw new Error(`端侧缓存缺失：${resultId}（目录可能被清理，请重新生成或恢复缓存）`);
  }
}

export function revokeClientBlobUrl(resultId: string) {
  const url = blobUrlCache.get(resultId);
  if (url) {
    URL.revokeObjectURL(url);
    blobUrlCache.delete(resultId);
  }
}
