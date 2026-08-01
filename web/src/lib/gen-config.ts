export type ImageQuality = "auto" | "high" | "medium" | "low";

export type GenConfig = {
  quality: ImageQuality;
  width: number;
  height: number;
  count: number;
  transparent_bg: boolean;
  align_16: boolean;
  polish_factor: number;
};

export const DEFAULT_GEN_CONFIG: GenConfig = {
  quality: "auto",
  width: 1024,
  height: 1024,
  count: 1,
  transparent_bg: false,
  align_16: true,
  polish_factor: 0,
};

export type AspectPreset = {
  id: string;
  label: string;
  w: number;
  h: number;
};

export const ASPECT_PRESETS: AspectPreset[] = [
  { id: "1:1", label: "1:1", w: 1024, h: 1024 },
  { id: "3:2", label: "3:2", w: 1536, h: 1024 },
  { id: "2:3", label: "2:3", w: 1024, h: 1536 },
  { id: "4:3", label: "4:3", w: 1365, h: 1024 },
  { id: "3:4", label: "3:4", w: 1024, h: 1365 },
  { id: "16:9", label: "16:9", w: 1792, h: 1024 },
  { id: "9:16", label: "9:16", w: 1024, h: 1792 },
  { id: "1:1_2k", label: "1:1(2k)", w: 2048, h: 2048 },
  { id: "16:9_2k", label: "16:9(2k)", w: 2752, h: 1536 },
  { id: "9:16_2k", label: "9:16(2k)", w: 1536, h: 2752 },
  { id: "16:9_4k", label: "16:9(4k)", w: 3840, h: 2160 },
  { id: "9:16_4k", label: "9:16(4k)", w: 2160, h: 3840 },
];

export const QUALITY_OPTIONS: { id: ImageQuality; label: string }[] = [
  { id: "auto", label: "自动" },
  { id: "high", label: "高" },
  { id: "medium", label: "中" },
  { id: "low", label: "低" },
];

export function snap16(n: number): number {
  return Math.max(16, Math.round(n / 16) * 16);
}

export function sizeString(cfg: GenConfig): string {
  const w = cfg.align_16 ? snap16(cfg.width) : cfg.width;
  const h = cfg.align_16 ? snap16(cfg.height) : cfg.height;
  return `${w}x${h}`;
}

export function snappedGenConfig(cfg: GenConfig): GenConfig {
  const [w, h] = sizeString(cfg).split("x").map(Number);
  return { ...cfg, width: w, height: h };
}
