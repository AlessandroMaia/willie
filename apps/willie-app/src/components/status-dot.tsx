import { TONE_DOT, type Tone } from "@/components/tone";
import { cn } from "@/lib/utils";

interface StatusDotProps {
  tone: Tone;
  /** Read by assistive technology; defaults to the tone itself. */
  label?: string;
  className?: string;
}

export function StatusDot({ tone, label, className }: StatusDotProps) {
  return (
    <span
      role="img"
      aria-label={label ?? tone}
      data-tone={tone}
      className={cn(
        "inline-block size-2 shrink-0 rounded-full",
        TONE_DOT[tone],
        className,
      )}
    />
  );
}
