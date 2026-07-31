/**
 * 主题 token 索引（运行时样式见 `src/styles/neo-theme.css`）。
 * Web 端等价于 Qt 的 QSS + QRC：CSS 变量 = 可调参数，组件 class = 样式选择器。
 */
export const NEO_THEME_TOKENS = {
  radius: ["--neo-radius-sm", "--neo-radius-md", "--neo-radius-lg"],
  color: ["--neo-ink", "--neo-muted", "--neo-border", "--neo-primary", "--neo-primary-deep"],
  shadow: ["--neo-shadow-idle", "--neo-shadow-hover", "--neo-shadow-active", "--neo-shadow-pressed"],
} as const;

export type NeoChoiceVariant = "segment" | "chip" | "pill";
