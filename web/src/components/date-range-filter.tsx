"use client";

type DateRangeFilterProps = {
  startDate: string;
  endDate: string;
  onChange: (startDate: string, endDate: string) => void;
  className?: string;
};

export function DateRangeFilter({ startDate, endDate, onChange, className }: DateRangeFilterProps) {
  return (
    <div className={className ?? "flex flex-wrap items-center gap-2"}>
      <input
        type="date"
        value={startDate}
        onChange={(e) => onChange(e.target.value, endDate)}
        className="neo-input h-8 rounded-lg px-2 text-sm text-[var(--neo-ink)]"
        aria-label="开始日期"
      />
      <span className="text-xs text-[var(--neo-muted)]">至</span>
      <input
        type="date"
        value={endDate}
        min={startDate || undefined}
        onChange={(e) => onChange(startDate, e.target.value)}
        className="neo-input h-8 rounded-lg px-2 text-sm text-[var(--neo-ink)]"
        aria-label="结束日期"
      />
    </div>
  );
}
