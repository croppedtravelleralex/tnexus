import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "@/lib/utils";
import type { NeoChoiceVariant } from "@/lib/theme";

const choiceVariants = cva("neo-choice", {
  variants: {
    variant: {
      segment: "neo-choice-segment",
      chip: "neo-choice-chip",
      pill: "neo-choice-pill",
    },
    active: {
      true: "neo-choice-active",
      false: "neo-choice-idle",
    },
  },
  defaultVariants: { variant: "segment", active: false },
});

export type ChoiceButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof choiceVariants> & {
    variant?: NeoChoiceVariant;
    active?: boolean;
  };

export function ChoiceButton({ className, variant, active, ...props }: ChoiceButtonProps) {
  return (
    <button type="button" className={cn(choiceVariants({ variant, active }), className)} {...props} />
  );
}

export function SegmentGroup({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("neo-segment-group", className)}>{children}</div>;
}
