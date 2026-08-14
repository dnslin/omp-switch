import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "../../lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap rounded-[var(--radius-md)] px-5 py-0 text-base font-medium transition-colors focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-[var(--color-primary-soft)] disabled:pointer-events-none",
  {
    variants: {
      variant: {
        primary: "border border-transparent bg-[var(--color-primary)] text-white enabled:hover:bg-[var(--color-primary-hover)]",
        secondary: "border border-[var(--color-border)] bg-[var(--color-bg-surface)] font-normal text-[var(--color-text-primary)] enabled:hover:border-[var(--color-border-strong)]",
      },
      size: {
        default: "min-h-[var(--control-height)]",
        setup: "min-h-[52px] px-8 text-[17px]",
      },
      disabledAppearance: {
        muted: "disabled:opacity-50",
        stable: "disabled:opacity-100",
      },
    },
    compoundVariants: [
      { variant: "primary", disabledAppearance: "stable", className: "disabled:bg-[var(--color-primary)]" },
      { variant: "secondary", disabledAppearance: "stable", className: "disabled:border-[var(--color-border)]" },
    ],
    defaultVariants: { variant: "primary", size: "default", disabledAppearance: "muted" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, disabledAppearance, asChild = false, ...props }, ref) => {
    const Component = asChild ? Slot : "button";
    return <Component ref={ref} className={cn(buttonVariants({ variant, size, disabledAppearance, className }))} {...props} />;
  },
);
Button.displayName = "Button";
