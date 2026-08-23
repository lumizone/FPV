import type { ReactNode } from "react";

/**
 * OrnamentDivider — gold-accented scene/chapter divider.
 * Used between narrative sections, world detail sections, and
 * panel content breaks. Matches the website's "✧" decorative
 * separator from firstpersonviewpoint.com.
 */
export function OrnamentDivider({ className = "" }: { className?: string }) {
  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <div className="h-px flex-1 bg-gradient-to-r from-transparent via-[var(--color-separator)]/30 to-transparent" />
      <span className="text-[10px] text-[var(--color-label-tertiary)] font-medium tracking-widest uppercase shrink-0 select-none">
        ✦
      </span>
      <div className="h-px flex-1 bg-gradient-to-r from-transparent via-[var(--color-separator)]/30 to-transparent" />
    </div>
  );
}

/**
 * SectionHeading — Cinzel heading with gold underline accent.
 * For panel section headers (Codex, Cast, Chronicles, etc.).
 */
export function SectionHeading({
  icon,
  label,
  count,
  className = "",
}: {
  icon?: ReactNode;
  label: string;
  count?: number;
  className?: string;
}) {
  return (
    <div className={`flex items-center gap-1.5 mb-2 ${className}`}>
      {icon && <span className="text-[var(--color-accent)] shrink-0">{icon}</span>}
      <h4 className="text-[11px] uppercase tracking-[0.08em] text-[var(--color-label-tertiary)] font-medium">
        {label}
      </h4>
      {count !== undefined && (
        <span className="text-[9px] text-[var(--color-label-quaternary)] ml-auto">
          {count}
        </span>
      )}
    </div>
  );
}

/**
 * GlassCard — elevated card with backdrop blur and optional gold border.
 * Thin wrapper over the existing `.surface-elevated` CSS class, adding
 * the book-like depth variant when `accent` is true.
 */
export function GlassCard({
  children,
  accent = false,
  className = "",
}: {
  children: ReactNode;
  accent?: boolean;
  className?: string;
}) {
  return (
    <div
      className={`surface-elevated ${
        accent ? "border-[var(--color-accent)]/15" : ""
      } ${className}`}
    >
      {children}
    </div>
  );
}
