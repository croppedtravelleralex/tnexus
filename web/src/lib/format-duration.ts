export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms / 100)) / 10}秒`;
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec}秒`;
  const min = Math.floor(sec / 60);
  const rem = sec % 60;
  return `${min}分${rem.toString().padStart(2, "0")}秒`;
}
