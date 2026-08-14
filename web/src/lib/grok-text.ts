/** 剥离 grok.com 上游专有标签（`<grok:render>` 等），避免透传到对话气泡。 */

const COMPLETE_TAG =
  /<grok:[A-Za-z0-9_.-]+(?:\s[^>]*)?\/>|<grok:([A-Za-z0-9_.-]+)(?:\s[^>]*)?>[\s\S]*?<\/grok:\1>/gi;

export function stripGrokMarkup(input: string): string {
  let stripped = input.replace(COMPLETE_TAG, "");
  const lastOpen = stripped.lastIndexOf("<grok:");
  if (lastOpen >= 0) {
    const tail = stripped.slice(lastOpen);
    if (!/<\/grok:[A-Za-z0-9_.-]+>/.test(tail) || !tail.includes(">")) {
      stripped = stripped.slice(0, lastOpen);
    }
  }
  return stripped;
}

/** 对话气泡展示用耗时。 */
export function formatReplyDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const sec = ms / 1000;
  if (sec < 10) return `${sec.toFixed(1)}s`;
  return `${Math.round(sec)}s`;
}

/** 用户明显在要图，而不是纯文本对话。 */
export function looksLikeImaginePrompt(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  return /生成.{0,8}(图|图片|照片|壁纸|插画)|画一[张幅]|帮我画|imagine\b|generate (an? )?(image|picture)|draw (me |an? )/i.test(
    t,
  );
}
