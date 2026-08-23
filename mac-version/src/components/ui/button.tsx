import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const button = cva(
  "no-drag inline-flex items-center justify-center gap-1.5 font-medium select-none whitespace-nowrap transition-[background,color,box-shadow] duration-100 disabled:opacity-40 disabled:pointer-events-none focus-visible:outline-none",
  {
    variants: {
      variant: {
        primary:
          "bg-[var(--color-accent)] text-black font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_92%,white)] active:bg-[color-mix(in_srgb,var(--color-accent)_88%,black)] shadow-[inset_0_1px_0_rgba(255,255,255,0.25),0_1px_2px_rgba(0,0,0,0.15)]",
        secondary:
          "bg-[var(--color-fill-quaternary)] text-[var(--color-label-primary)] hover:bg-[var(--color-fill-tertiary)] active:bg-[var(--color-fill-secondary)] border border-[var(--color-separator)]",
        ghost:
          "bg-transparent text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] active:bg-[var(--color-fill-tertiary)]",
        danger:
          "bg-[var(--color-system-pink)] text-white hover:brightness-95 active:brightness-90",
        link:
          "bg-transparent text-[var(--color-system-blue)] hover:underline underline-offset-2 px-0 h-auto",
      },
      size: {
        sm: "h-7 px-2.5 text-[12px] rounded-[var(--radius-sm)]",
        md: "h-8 px-3 text-[13px] rounded-[var(--radius-md)]",
        lg: "h-10 px-4 text-[14px] rounded-[var(--radius-md)]",
        icon: "h-8 w-8 rounded-[var(--radius-md)]",
        iconSm: "h-7 w-7 rounded-[var(--radius-sm)]",
      },
    },
    defaultVariants: {
      variant: "secondary",
      size: "md",
    },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof button> {}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => (
    <button
      ref={ref}
      className={cn(button({ variant, size }), className)}
      {...props}
    />
  )
);
Button.displayName = "Button";
