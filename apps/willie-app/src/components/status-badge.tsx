import type { ComponentProps } from "react";
import { TONE_SURFACE, type Tone } from "@/components/tone";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

interface StatusBadgeProps extends ComponentProps<typeof Badge> {
  tone: Tone;
}

/** The generated Badge with the design system's tone on it: a tinted
 * surface and matching text, the one shape every state pill shares. */
export function StatusBadge({ tone, className, ...props }: StatusBadgeProps) {
  return (
    <Badge
      variant="outline"
      data-tone={tone}
      className={cn("border-transparent", TONE_SURFACE[tone], className)}
      {...props}
    />
  );
}
