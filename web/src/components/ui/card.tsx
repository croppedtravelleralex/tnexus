import { cn } from "@/lib/utils";

export function Card({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("rounded-lg border border-zinc-200 bg-white shadow-sm", className)}>
      {children}
    </div>
  );
}
