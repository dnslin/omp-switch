import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "../../lib/utils";

const buttonVariants = cva("app-button", {
  variants: {
    variant: {
      primary: "app-button--primary",
      secondary: "app-button--secondary",
    },
    size: {
      default: "app-button--default",
      setup: "app-button--setup",
    },
    disabledAppearance: {
      muted: "app-button--disabled-muted",
      stable: "app-button--disabled-stable",
    },
  },
  defaultVariants: { variant: "primary", size: "default", disabledAppearance: "muted" },
});

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
