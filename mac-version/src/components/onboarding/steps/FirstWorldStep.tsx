import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import type { World } from "@/lib/tauri";

interface Props {
  worlds: World[] | null;
  onPick: (world: World) => void;
  onSkip: () => void;
  onShowAll?: () => void;
}

/// One world per genre, in the order the seed file introduces them.
///
/// The seeded library is 19 worlds across 5 genres (fantasy 4, scifi 4,
/// horror 3, manga 4, romance 4). Showing one of each reads as range;
/// showing all 19 on a first-run screen reads as homework.
function onePerGenre(worlds: World[]): World[] {
  const seen = new Set<string>();
  const picked: World[] = [];
  for (const w of worlds) {
    const g = w.genre.toLowerCase();
    if (seen.has(g)) continue;
    seen.add(g);
    picked.push(w);
  }
  return picked;
}

/// Second screen of the first run — and the reason the flow is shaped this
/// way at all.
///
/// The chat model is 6.6-22 GB depending on RAM tier: minutes of waiting.
/// Rather than park the user in front of a progress bar, this screen spends
/// that time on the one thing FPV has that a general local-LLM runner does
/// not — hand-written worlds. The download reports underneath, with real
/// byte counts rather than an indeterminate spinner.
export function FirstWorldStep({ worlds, onPick, onSkip, onShowAll }: Props) {
  const { t } = useTranslation();
  const download = useApp((s) => s.model_download);

  const featured = useMemo(
    () => (worlds ? onePerGenre(worlds) : []),
    [worlds]
  );

  const remaining = worlds ? worlds.length - featured.length : 0;

  const pct =
    download && download.total > 0
      ? Math.min(100, Math.round((download.completed / download.total) * 100))
      : null;

  return (
    <div className="max-w-2xl mx-auto flex flex-col gap-6 py-2">
      <div className="w-full h-32 rounded-2xl overflow-hidden border border-[var(--color-separator)]/30">
        <img
          src="/onboarding/genre-portals.webp"
          alt=""
          aria-hidden="true"
          className="w-full h-full object-cover"
        />
      </div>
      <header className="text-center">
        <h2 className="font-display text-[19px] text-[var(--color-label-primary)] text-balance">
          {t("onboarding.world.title", "Where does it begin?")}
        </h2>
        {remaining > 0 && (
          <p className="mt-2 font-serif text-[12.5px] text-[var(--color-label-secondary)]">
            {t("onboarding.world.subtitle", {
              defaultValue:
                "{{shown}} to start with. {{rest}} more are already on your Mac.",
              shown: featured.length,
              rest: remaining,
            })}
          </p>
        )}
      </header>

      {worlds === null ? (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2.5" aria-hidden="true">
          {Array.from({ length: 6 }).map((_, i) => (
            <div
              key={i}
              className="h-[104px] rounded-lg border border-[var(--color-separator)] bg-[var(--color-fill-quaternary)] animate-pulse"
            />
          ))}
        </div>
      ) : (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2.5">
          {featured.map((w) => (
            <button
              key={w.id}
              onClick={() => onPick(w)}
              className={cn(
                "group text-left rounded-lg overflow-hidden border transition-colors",
                "border-[var(--color-separator)] bg-[var(--color-fill-quaternary)]",
                "hover:border-[var(--color-label-tertiary)]",
                "focus-visible:outline-none focus-visible:border-[var(--color-accent)]"
              )}
            >
              {/* The seed file gives every world its own accent_color; using
                  it keeps the row legible as five distinct worlds. */}
              <span
                className="block h-9"
                style={{
                  background: `linear-gradient(135deg, ${w.accent_color ?? "var(--color-accent)"}55, ${w.accent_color ?? "var(--color-accent)"}12)`,
                }}
              />
              <span className="block px-2.5 pt-2 pb-2.5">
                <span
                  className="block text-[8px] uppercase tracking-[0.14em] mb-1"
                  style={{ color: w.accent_color ?? "var(--color-accent)" }}
                >
                  {w.genre}
                </span>
                <span className="block font-display text-[12px] leading-tight text-[var(--color-label-primary)]">
                  {w.name}
                </span>
                {w.description && (
                  <span className="block mt-1 text-[9.5px] leading-snug text-[var(--color-label-tertiary)] line-clamp-2">
                    {w.description}
                  </span>
                )}
              </span>
            </button>
          ))}

          <button
            onClick={worlds.length && onShowAll ? onShowAll : onSkip}
            className={cn(
              "rounded-lg border border-dashed border-[var(--color-separator)]",
              "text-[10px] leading-snug px-3 py-4 text-[var(--color-label-secondary)]",
              "hover:border-[var(--color-label-tertiary)] transition-colors",
              "focus-visible:outline-none focus-visible:border-[var(--color-accent)]"
            )}
          >
            {worlds.length
              ? t("onboarding.world.show_all", {
                  defaultValue: "Show me all {{count}} →",
                  count: worlds.length,
                })
              : t("onboarding.world.skip", "Skip →")}
          </button>
        </div>
      )}

      {/* Engine download. Only shown while it is actually running — a
          finished or absent pull leaves the screen quiet. */}
      {download && download.phase === "active" && (
        <div className="border-t border-[var(--color-separator)] pt-3 flex flex-col gap-1.5">
          <div className="flex items-baseline justify-between text-[10px] text-[var(--color-label-secondary)]">
            <span>{t("onboarding.world.preparing", "Preparing your narrator")}</span>
            <span className="tabular-nums text-[var(--color-label-primary)]">
              {download.total > 0
                ? `${(download.completed / 1e9).toFixed(1)} / ${(download.total / 1e9).toFixed(1)} GB`
                : download.status}
            </span>
          </div>
          <div className="h-[3px] rounded-sm bg-[var(--color-fill-quaternary)] overflow-hidden">
            <div
              className={cn(
                "h-full bg-[var(--color-accent)] rounded-sm transition-[width] duration-300",
                pct === null && "w-1/3 animate-pulse"
              )}
              style={pct !== null ? { width: `${pct}%` } : undefined}
            />
          </div>
        </div>
      )}
    </div>
  );
}
