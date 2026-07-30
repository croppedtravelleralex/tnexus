export type StudioLayoutPrefs = {
  columnWidths: [number, number, number];
};

export type UserPreferences = {
  studio_layout?: StudioLayoutPrefs;
};
export const DEFAULT_COLUMN_WIDTHS: [number, number, number] = [260, 420, 560];

export const LAYOUT_STORAGE_KEY = "tnexus:studio-layout";

export const MIN_COLUMN_WIDTH = 180;

export function normalizeWidths(widths: [number, number, number], total: number): [number, number, number] {
  const sum = widths[0] + widths[1] + widths[2];
  if (sum <= 0) return DEFAULT_COLUMN_WIDTHS;
  const scale = total / sum;
  return [
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[0] * scale)),
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[1] * scale)),
    Math.max(MIN_COLUMN_WIDTH, Math.round(widths[2] * scale)),
  ];
}
