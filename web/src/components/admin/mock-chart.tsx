"use client";

/** 占位折线图 — UI 验收用，后续接真实 API */
export function MockLineChart({ labels, series }: { labels: string[]; series: number[] }) {
  const max = Math.max(...series, 1);
  const w = 640;
  const h = 160;
  const pad = 8;
  const points = series
    .map((v, i) => {
      const x = pad + (i / Math.max(series.length - 1, 1)) * (w - pad * 2);
      const y = h - pad - (v / max) * (h - pad * 2);
      return `${x},${y}`;
    })
    .join(" ");

  return (
    <div className="w-full">
      <svg viewBox={`0 0 ${w} ${h}`} className="h-40 w-full text-zinc-300">
        <polyline
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinejoin="round"
          strokeLinecap="round"
          points={points}
          className="text-zinc-400"
        />
        <polyline
          fill="none"
          stroke="url(#chartGrad)"
          strokeWidth="2.5"
          strokeLinejoin="round"
          strokeLinecap="round"
          points={points}
        />
        <defs>
          <linearGradient id="chartGrad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="#71717a" />
            <stop offset="100%" stopColor="#18181b" />
          </linearGradient>
        </defs>
      </svg>
      <div className="mt-2 flex justify-between text-[10px] text-zinc-400">
        {labels.filter((_, i) => i % Math.ceil(labels.length / 6) === 0).map((l) => (
          <span key={l}>{l}</span>
        ))}
      </div>
    </div>
  );
}
