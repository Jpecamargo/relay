import type { ComponentProps } from "react";
import styles from "./Button.module.css";

export interface ButtonProps extends ComponentProps<"button"> {
  variant?: "default" | "primary" | "danger" | "ghost";
}

export function Button({ variant = "default", className, type = "button", ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      className={[styles.button, styles[variant], className].filter(Boolean).join(" ")}
      {...rest}
    />
  );
}
