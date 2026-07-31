export default function ConsoleLoading() {
  return (
    <div className="flex min-h-[40vh] flex-col gap-4 px-4 py-6 sm:px-6">
      <div className="h-7 w-40 animate-pulse rounded-lg bg-[var(--neo-surface-muted)]" />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <div key={i} className="neo-card h-24 animate-pulse bg-[var(--neo-surface-muted)]/60" />
        ))}
      </div>
      <div className="neo-card min-h-[280px] flex-1 animate-pulse bg-[var(--neo-surface-muted)]/40" />
    </div>
  );
}
