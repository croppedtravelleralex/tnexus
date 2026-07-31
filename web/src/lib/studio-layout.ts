/** 全局 Studio 三列比例（左 20% / 中 30% / 右 50%），与产品默认布局一致 */
export const DEFAULT_COLUMN_RATIOS: [number, number, number] = [0.2, 0.3, 0.5];

const LAYOUT_STORAGE_KEY = "tnexus_studio_column_ratios";

export const MIN_COLUMN_WIDTH = 180;

/** 兼容旧引用；以 1500px 容器估算的像素宽度 */
export const DEFAULT_COLUMN_WIDTHS: [number, number, number] = [300, 450, 750];

export function normalizeWidths(widths: [number, number, number], total: number): [number, number, number] {
  const sum = widths[0] + widths[1] + widths[2];
  if (sum <= 0) return defaultColumnWidths(total);
  const scale = total / sum;
  return [
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[0] * scale)),
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[1] * scale)),
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[2] * scale)),
  ];
}

/** 按全局比例计算当前容器下的列宽 */
export function defaultColumnWidths(containerInnerWidth: number): [number, number, number] {
  const total = Math.max(containerInnerWidth, MIN_COLUMN_WIDTH * 3);
  const raw: [number, number, number] = [
    Math.round(DEFAULT_COLUMN_RATIOS[0] * total),
    Math.round(DEFAULT_COLUMN_RATIOS[1] * total),
    Math.round(DEFAULT_COLUMN_RATIOS[2] * total),
  ];
  return normalizeWidths(raw, total);
}

export function loadSavedColumnRatios(): [number, number, number] | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as number[];
    if (parsed.length !== 3 || parsed.some((n) => !Number.isFinite(n) || n <= 0)) return null;
    const sum = parsed[0] + parsed[1] + parsed[2];
    if (sum <= 0) return null;
    return [parsed[0] / sum, parsed[1] / sum, parsed[2] / sum] as [number, number, number];
  } catch {
    return null;
  }
}

export function saveColumnRatios(widths: [number, number, number]) {
  if (typeof window === "undefined") return;
  const sum = widths[0] + widths[1] + widths[2];
  if (sum <= 0) return;
  try {
    localStorage.setItem(
      LAYOUT_STORAGE_KEY,
      JSON.stringify([widths[0] / sum, widths[1] / sum, widths[2] / sum]),
    );
  } catch {
    // ignore quota
  }
}

export function columnWidthsFromRatios(containerInnerWidth: number, ratios: [number, number, number]): [number, number, number] {
  const total = Math.max(containerInnerWidth, MIN_COLUMN_WIDTH * 3);
  const raw: [number, number, number] = [
    Math.round(ratios[0] * total),
    Math.round(ratios[1] * total),
    Math.round(ratios[2] * total),
  ];
  return normalizeWidths(raw, total);
}
