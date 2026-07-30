import type { FactorPoint } from "@/lib/api";

export type StylePreset = {
  name: string;
  director: FactorPoint;
  render: FactorPoint;
};

export const STYLE_PRESETS: StylePreset[] = [
  { name: "电影感", director: { x: 0.35, y: 0.75 }, render: { x: 0.7, y: 0.85 } },
  { name: "产品图", director: { x: 0.85, y: 0.25 }, render: { x: 0.6, y: 0.2 } },
  { name: "概念艺术", director: { x: 0.2, y: 0.55 }, render: { x: 0.45, y: 0.65 } },
  { name: "赛博朋克", director: { x: 0.55, y: 0.8 }, render: { x: 0.85, y: 0.9 } },
  { name: "日系插画", director: { x: 0.3, y: 0.65 }, render: { x: 0.55, y: 0.5 } },
  { name: "写实人像", director: { x: 0.75, y: 0.45 }, render: { x: 0.65, y: 0.35 } },
  { name: "水墨国风", director: { x: 0.15, y: 0.6 }, render: { x: 0.25, y: 0.55 } },
  { name: "复古胶片", director: { x: 0.4, y: 0.7 }, render: { x: 0.5, y: 0.75 } },
  { name: "极简主义", director: { x: 0.9, y: 0.2 }, render: { x: 0.15, y: 0.15 } },
  { name: "奇幻史诗", director: { x: 0.25, y: 0.85 }, render: { x: 0.8, y: 0.8 } },
  { name: "商业广告", director: { x: 0.8, y: 0.35 }, render: { x: 0.7, y: 0.4 } },
  { name: "梦幻童话", director: { x: 0.2, y: 0.8 }, render: { x: 0.4, y: 0.7 } },
];
