import * as React from "react";
import { cn } from "@/lib/utils";

export function Input({ className, ...props }: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        "neo-input flex h-9 w-full px-3 text-sm text-[var(--neo-ink)] placeholder:text-[var(--neo-muted)] focus-visible:outline-none",
        className,
      )}
      {...props}
    />
  );
}

export function Textarea({ className, ...props }: React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      className={cn(
        "neo-input flex min-h-[100px] w-full px-3 py-2 text-sm text-[var(--neo-ink)] placeholder:text-[var(--neo-muted)] focus-visible:outline-none",
        className,
      )}
      {...props}
    />
  );
}

export function Label({ className, ...props }: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return (
    <label className={cn("text-sm font-medium text-[var(--neo-ink)] text-shadow-bl-sm", className)} {...props} />
  );
}
