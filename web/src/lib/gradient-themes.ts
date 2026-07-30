export type GradientThemeId = "moss" | "glass" | "ember" | "slate";

export const GRADIENT_THEMES: {
  id: GradientThemeId;
  label: string;
  description: string;
  preview: string;
}[] = [
  {
    id: "moss",
    label: "苔藓微光",
    description: "低对比墨绿，顶部极淡高光，适合长时间使用",
    preview: "linear-gradient(180deg, #2a5248 0%, #163b32 100%)",
  },
  {
    id: "glass",
    label: "玻璃拟态",
    description: "半透明磨砂感，几乎无白绿跳变",
    preview: "linear-gradient(135deg, rgba(142,182,155,0.25) 0%, rgba(22,59,50,0.9) 100%)",
  },
  {
    id: "ember",
    label: "暖绿琥珀",
    description: "偏暖的橄榄绿渐变，按钮有轻微金色边",
    preview: "linear-gradient(160deg, #3d5a4a 0%, #1a3328 60%, #0f241c 100%)",
  },
  {
    id: "slate",
    label: "冷墨石板",
    description: "冷色调深绿，扁平克制，无强烈高光",
    preview: "linear-gradient(180deg, #1e3d36 0%, #0b2b26 100%)",
  },
];

export const GRADIENT_STORAGE_KEY = "tnexus-gradient-theme";
