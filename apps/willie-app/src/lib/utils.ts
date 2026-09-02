import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merges class lists the way the generated primitives expect: `clsx`
 * for conditionals, `tailwind-merge` so a later utility wins over an
 * earlier one of the same group instead of both being emitted. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
