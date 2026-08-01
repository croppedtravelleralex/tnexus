import type { FactorPoint } from "@/lib/api";

export type StyleVariant = {
  /** 展示名，如「电影感 · 霓虹雨夜」 */
  name: string;
  /** 注入构思/生图的英文风格短语 */
  promptHint: string;
  director: FactorPoint;
  render: FactorPoint;
};

export type StyleCategory = {
  id: string;
  name: string;
  variants: StyleVariant[];
};

export const STYLE_CATEGORIES: StyleCategory[] = [
  {
    id: "cinematic",
    name: "电影感",
    variants: [
      { name: "霓虹雨夜", promptHint: "neon-lit rainy night, cinematic anamorphic lens, moody atmosphere", director: { x: 0.35, y: 0.8 }, render: { x: 0.55, y: 0.9 } },
      { name: "黄金时刻", promptHint: "golden hour backlight, warm cinematic color grade, shallow depth of field", director: { x: 0.4, y: 0.75 }, render: { x: 0.6, y: 0.85 } },
      { name: "黑白胶片", promptHint: "black and white film noir, high contrast, grain texture", director: { x: 0.3, y: 0.7 }, render: { x: 0.45, y: 0.8 } },
      { name: "史诗广角", promptHint: "epic wide establishing shot, dramatic scale, atmospheric haze", director: { x: 0.25, y: 0.85 }, render: { x: 0.75, y: 0.88 } },
    ],
  },
  {
    id: "product",
    name: "产品图",
    variants: [
      { name: "纯白底", promptHint: "clean white studio background, product hero shot, soft shadow", director: { x: 0.85, y: 0.25 }, render: { x: 0.15, y: 0.15 } },
      { name: "大理石台面", promptHint: "marble surface product photography, premium lifestyle styling", director: { x: 0.8, y: 0.3 }, render: { x: 0.5, y: 0.35 } },
      { name: "悬浮展示", promptHint: "floating product, minimal gradient backdrop, commercial advertising", director: { x: 0.82, y: 0.28 }, render: { x: 0.4, y: 0.45 } },
      { name: "场景化", promptHint: "lifestyle product in context, natural ambient light, brand storytelling", director: { x: 0.75, y: 0.4 }, render: { x: 0.55, y: 0.4 } },
    ],
  },
  {
    id: "concept",
    name: "概念艺术",
    variants: [
      { name: "科幻概念", promptHint: "sci-fi concept art, matte painting, futuristic architecture", director: { x: 0.2, y: 0.55 }, render: { x: 0.65, y: 0.7 } },
      { name: "奇幻场景", promptHint: "fantasy environment concept, magical lighting, painterly style", director: { x: 0.22, y: 0.8 }, render: { x: 0.7, y: 0.75 } },
      { name: "角色设定", promptHint: "character design sheet, turnaround-friendly, clear silhouette", director: { x: 0.7, y: 0.5 }, render: { x: 0.55, y: 0.55 } },
      { name: "废土世界", promptHint: "post-apocalyptic wasteland, dusty atmosphere, ruins", director: { x: 0.18, y: 0.65 }, render: { x: 0.6, y: 0.82 } },
    ],
  },
  {
    id: "cyberpunk",
    name: "赛博朋克",
    variants: [
      { name: "霓虹街巷", promptHint: "cyberpunk street, neon signs, rain reflections, blade runner mood", director: { x: 0.55, y: 0.8 }, render: { x: 0.85, y: 0.9 } },
      { name: "全息广告", promptHint: "holographic billboards, dense urban night, magenta cyan palette", director: { x: 0.5, y: 0.75 }, render: { x: 0.8, y: 0.85 } },
      { name: "机甲工业", promptHint: "mecha industrial cyberpunk, hard surface details, smoke and sparks", director: { x: 0.6, y: 0.55 }, render: { x: 0.9, y: 0.75 } },
    ],
  },
  {
    id: "anime",
    name: "日系插画",
    variants: [
      { name: "清新日常", promptHint: "soft anime illustration, pastel palette, slice of life", director: { x: 0.3, y: 0.65 }, render: { x: 0.4, y: 0.45 } },
      { name: "热血战斗", promptHint: "dynamic anime action pose, speed lines, vivid colors", director: { x: 0.45, y: 0.85 }, render: { x: 0.7, y: 0.8 } },
      { name: "吉卜力风", promptHint: "studio ghibli inspired, hand-painted backgrounds, warm nostalgia", director: { x: 0.25, y: 0.7 }, render: { x: 0.5, y: 0.55 } },
    ],
  },
  {
    id: "portrait",
    name: "写实人像",
    variants: [
      { name: "棚拍肖像", promptHint: "studio portrait photography, catchlight in eyes, 85mm lens", director: { x: 0.75, y: 0.45 }, render: { x: 0.55, y: 0.4 } },
      { name: "街头抓拍", promptHint: "candid street portrait, natural light, documentary feel", director: { x: 0.65, y: 0.55 }, render: { x: 0.45, y: 0.35 } },
      { name: "时尚大片", promptHint: "high fashion editorial portrait, bold styling, magazine cover", director: { x: 0.7, y: 0.6 }, render: { x: 0.65, y: 0.7 } },
    ],
  },
  {
    id: "ink",
    name: "水墨国风",
    variants: [
      { name: "山水意境", promptHint: "traditional Chinese ink wash landscape, misty mountains, negative space", director: { x: 0.15, y: 0.6 }, render: { x: 0.2, y: 0.55 } },
      { name: "工笔花鸟", promptHint: "gongbi fine brush flower and bird, delicate lines, silk texture", director: { x: 0.8, y: 0.35 }, render: { x: 0.35, y: 0.3 } },
      { name: "写意人物", promptHint: "expressive ink figure, flowing brush strokes, rice paper", director: { x: 0.2, y: 0.7 }, render: { x: 0.3, y: 0.5 } },
    ],
  },
  {
    id: "minimal",
    name: "极简主义",
    variants: [
      { name: "留白构图", promptHint: "minimalist composition, abundant negative space, single focal subject", director: { x: 0.9, y: 0.2 }, render: { x: 0.1, y: 0.15 } },
      { name: "几何色块", promptHint: "geometric minimalism, flat color blocks, bauhaus influence", director: { x: 0.85, y: 0.25 }, render: { x: 0.2, y: 0.2 } },
      { name: "单色静物", promptHint: "monochrome still life, subtle tonal variation, zen simplicity", director: { x: 0.88, y: 0.22 }, render: { x: 0.12, y: 0.18 } },
    ],
  },
  {
    id: "commercial",
    name: "商业广告",
    variants: [
      { name: "品牌 KV", promptHint: "brand key visual, polished commercial lighting, premium feel", director: { x: 0.8, y: 0.35 }, render: { x: 0.7, y: 0.4 } },
      { name: "电商主图", promptHint: "e-commerce hero image, crisp details, conversion-focused framing", director: { x: 0.85, y: 0.3 }, render: { x: 0.55, y: 0.25 } },
      { name: "节日促销", promptHint: "festive promotional visual, vibrant colors, celebratory mood", director: { x: 0.6, y: 0.75 }, render: { x: 0.65, y: 0.6 } },
    ],
  },
  {
    id: "fantasy",
    name: "梦幻童话",
    variants: [
      { name: "童话绘本", promptHint: "storybook illustration, whimsical fairy tale, soft glow", director: { x: 0.2, y: 0.8 }, render: { x: 0.4, y: 0.7 } },
      { name: "魔法森林", promptHint: "enchanted forest, bioluminescent plants, dreamy atmosphere", director: { x: 0.25, y: 0.85 }, render: { x: 0.55, y: 0.75 } },
      { name: "星空梦境", promptHint: "starry dreamscape, celestial fantasy, ethereal light", director: { x: 0.18, y: 0.78 }, render: { x: 0.35, y: 0.82 } },
    ],
  },
];

/** @deprecated 使用 STYLE_CATEGORIES */
export type StylePreset = StyleVariant & { name: string };

export const STYLE_PRESETS: StylePreset[] = STYLE_CATEGORIES.flatMap((c) =>
  c.variants.map((v) => ({ ...v, name: `${c.name} · ${v.name}` })),
);

export function stylePresetLabel(categoryName: string, variantName: string) {
  return `${categoryName} · ${variantName}`;
}
