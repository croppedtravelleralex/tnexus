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
