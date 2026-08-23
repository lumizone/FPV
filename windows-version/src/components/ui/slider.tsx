import { cn } from "@/lib/utils";

export interface SliderProps {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  leftLabel?: string;
  rightLabel?: string;
  className?: string;
  /** Accessible name for the underlying `<input type="range">` — the
   *  visible caption above it (if any) isn't programmatically associated
   *  with the control, so a screen reader otherwise announces an
   *  unlabeled slider. */
  ariaLabel?: string;
}

/** macOS-style horizontal slider with optional bipolar labels. */
export function Slider({
  value,
  onChange,
  min = 0,
  max = 1,
  step = 0.05,
  leftLabel,
  rightLabel,
  className,
  ariaLabel,
}: SliderProps) {
  const pct = Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100));
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      {(leftLabel || rightLabel) && (
        <div className="flex justify-between text-[11px] text-[var(--color-label-secondary)] px-0.5">
          <span>{leftLabel}</span>
          <span>{rightLabel}</span>
        </div>
      )}
      {/* The visual thumb used to chase the value via `transition-[left]`,
         which felt laggy on drag (CSS interpolated the position instead
         of tracking the mouse 1:1). The hidden <input type=range> now
         drives a `--pct` custom property updated on every change event —
         no transition, no rubber-banding. Increased the click target to
         h-8 (32 px) so the thumb is actually grabbable on a hi-DPI
         display. */}
      <div className="relative h-8 flex items-center no-drag select-none">
        <div className="absolute inset-x-0 h-1.5 rounded-full bg-[var(--color-fill-quaternary)]" />
        <div
          className="absolute h-1.5 rounded-full bg-[var(--color-system-blue)]"
          style={{ width: `${pct}%` }}
        />
        <input
          type="range"
          aria-label={ariaLabel}
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
          className="absolute inset-0 h-full w-full opacity-0 cursor-grab active:cursor-grabbing"
          style={{ touchAction: "none" }}
        />
        <div
          className="absolute h-5 w-5 rounded-full bg-[var(--color-label-primary)] shadow-[0_1px_3px_rgba(0,0,0,0.25),0_2px_6px_rgba(0,0,0,0.12)] border border-[var(--color-separator)] -translate-x-1/2 pointer-events-none"
          style={{ left: `${pct}%` }}
        />
      </div>
    </div>
  );
}
