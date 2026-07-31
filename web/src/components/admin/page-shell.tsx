import { cn } from "@/lib/utils";

export function PageShell({
  title,
  subtitle,
  badge,
  actions,
  children,
  className,
  fullBleed,
}: {
  title: string;
  subtitle?: string;
  badge?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  fullBleed?: boolean;
}) {
  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", fullBleed ? "" : "px-4 py-4 sm:px-6 sm:py-5")}>
      {!fullBleed ? (
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="text-lg font-semibold tracking-tight text-[var(--neo-ink)] sm:text-xl text-shadow-bl">{title}</h1>
              {badge ? (
                <span className="rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-[10px] font-medium text-amber-700">
                  {badge}
                </span>
              ) : null}
            </div>
            {subtitle ? <p className="mt-1 text-sm text-[var(--neo-muted)]">{subtitle}</p> : null}
          </div>
          {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
      <div className={cn("min-h-0 flex-1", className)}>{children}</div>
    </div>
  );
}

export function ElevatedCard({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "neo-card",
        className,
      )}
    >
      {children}
    </div>
  );
}
