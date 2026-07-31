import { cn } from "@/lib/utils";

export function Badge({
  className,
  variant = "default",
  children,
}: {
  className?: string;
  variant?: "default" | "success" | "warning" | "muted" | "info" | "secondary" | "danger";
  children: React.ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-[11px] font-medium",
        variant === "default" && "bg-zinc-900 text-white",
        variant === "success" && "bg-emerald-50 text-emerald-700 ring-1 ring-emerald-200/60",
        variant === "warning" && "bg-amber-50 text-amber-700 ring-1 ring-amber-200/60",
        variant === "muted" && "bg-zinc-100 text-zinc-600",
        variant === "info" && "bg-sky-50 text-sky-700 ring-1 ring-sky-200/60",
        variant === "secondary" && "bg-stone-100 text-stone-700 ring-1 ring-stone-200/60",
        variant === "danger" && "bg-rose-50 text-rose-700 ring-1 ring-rose-200/60",
        className,
      )}
    >
      {children}
    </span>
  );
}
