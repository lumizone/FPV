import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";

interface Props {
  onContinue: () => void;
}

const STORY_TYPES = [
  { key: "epic", label: "Epic quests & adventure", icon: "⚔" },
  { key: "intrigue", label: "Political intrigue", icon: "👑" },
  { key: "drama", label: "Personal drama", icon: "🎭" },
  { key: "mystery", label: "Mystery & discovery", icon: "🔍" },
  { key: "dark", label: "Dark & atmospheric", icon: "🌑" },
  { key: "surprise", label: "Surprise me", icon: "✨" },
] as const;

const FREEDOM_LEVELS = [
  { key: "guided", label: "Guide me", sub: "A structured story with clear direction" },
  { key: "balanced", label: "Balanced", sub: "Surprises are welcome" },
  { key: "free", label: "Full freedom", sub: "I'll drive the narrative" },
] as const;

const NARRATIVE_STYLES = [
  { key: "literary", label: "Literary & descriptive", sub: "Rich prose, vivid detail" },
  { key: "fast", label: "Fast-paced & punchy", sub: "Action-driven, tight pacing" },
  { key: "cinematic", label: "Cinematic & visual", sub: "Scene-focused, atmospheric" },
] as const;

/**
 * PreferenceStep — collects narrative preferences between Welcome and
 * World pick. 2-3 quick questions that will shape the narrator's style
 * and help sort worlds during the next step.
 *
 * All answers are stored as preferences so `promptBuilder.ts` can read
 * them without a new backend command.
 */
export function PreferenceStep({ onContinue }: Props) {
  const { t } = useTranslation();
  const updatePreference = useApp((s) => s.updatePreference);

  const [storyType, setStoryType] = useState<string | null>(null);
  const [freedom, setFreedom] = useState<string | null>(null);
  const [style, setStyle] = useState<string | null>(null);

  function handleContinue() {
    if (storyType) updatePreference("narrative_theme", storyType);
    if (freedom) updatePreference("narrative_freedom", freedom);
    if (style) updatePreference("narrative_style", style);
    onContinue();
  }

  const canContinue = storyType !== null;

  return (
    <div className="max-w-lg mx-auto flex flex-col gap-8 py-6">
      {/* ── Question 1: Story type ─────────────────────────────── */}
      <fieldset>
        <legend className="font-serif text-[14px] leading-[1.6] text-[var(--color-label-primary)] text-center w-full mb-4 text-balance">
          {t(
            "onboarding.prefs.story_type",
            "What draws you into a story?"
          )}
        </legend>
        <div className="grid grid-cols-2 gap-2">
          {STORY_TYPES.map((opt) => {
            const active = storyType === opt.key;
            return (
              <button
                key={opt.key}
                type="button"
                onClick={() => setStoryType(opt.key)}
                className={`flex items-center gap-2 px-3 py-2.5 rounded-lg text-[12px] text-left transition-all border ${
                  active
                    ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-label-primary)]"
                    : "border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 text-[var(--color-label-secondary)] hover:border-[var(--color-separator)]/40 hover:text-[var(--color-label-primary)]"
                }`}
              >
                <span className="text-[14px] shrink-0">{opt.icon}</span>
                <span>{t(`onboarding.prefs.${opt.key}`, opt.label)}</span>
              </button>
            );
          })}
        </div>
      </fieldset>

      {/* ── Question 2: Freedom level ──────────────────────────── */}
      <fieldset>
        <legend className="font-serif text-[14px] leading-[1.6] text-[var(--color-label-primary)] text-center w-full mb-3 text-balance">
          {t(
            "onboarding.prefs.freedom",
            "How much freedom do you want?"
          )}
        </legend>
        <div className="flex flex-col gap-1.5">
          {FREEDOM_LEVELS.map((opt) => {
            const active = freedom === opt.key;
            return (
              <button
                key={opt.key}
                type="button"
                onClick={() => setFreedom(opt.key)}
                className={`flex items-center justify-between px-3 py-2.5 rounded-lg text-left transition-all border ${
                  active
                    ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)]"
                    : "border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 hover:border-[var(--color-separator)]/40"
                }`}
              >
                <span className={`text-[12px] font-medium ${active ? "text-[var(--color-label-primary)]" : "text-[var(--color-label-secondary)]"}`}>
                   {t(`onboarding.prefs.${opt.key}`, opt.label)}
                </span>
                <span className="text-[10.5px] text-[var(--color-label-tertiary)] hidden sm:inline">
                  {opt.sub}
                </span>
              </button>
            );
          })}
        </div>
      </fieldset>

      {/* ── Question 3: Narrative style ────────────────────────── */}
      <fieldset>
        <legend className="font-serif text-[14px] leading-[1.6] text-[var(--color-label-primary)] text-center w-full mb-3 text-balance">
          {t(
            "onboarding.prefs.style",
            "What narrative style?"
          )}
        </legend>
        <div className="flex flex-col gap-1.5">
          {NARRATIVE_STYLES.map((opt) => {
            const active = style === opt.key;
            return (
              <button
                key={opt.key}
                type="button"
                onClick={() => setStyle(opt.key)}
                className={`flex items-center justify-between px-3 py-2.5 rounded-lg text-left transition-all border ${
                  active
                    ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)]"
                    : "border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 hover:border-[var(--color-separator)]/40"
                }`}
              >
                <span className={`text-[12px] font-medium ${active ? "text-[var(--color-label-primary)]" : "text-[var(--color-label-secondary)]"}`}>
                  {t(`onboarding.prefs.${opt.key}`, opt.label)}
                </span>
                <span className="text-[10.5px] text-[var(--color-label-tertiary)]">
                  {opt.sub}
                </span>
              </button>
            );
          })}
        </div>
      </fieldset>

      {/* ── CTA ────────────────────────────────────────────────── */}
      <div className="flex flex-col items-center gap-3">
        <Button
          variant="primary"
          size="lg"
          className="px-8"
          onClick={handleContinue}
          disabled={!canContinue}
        >
          {t("onboarding.prefs.continue", "Continue")}
        </Button>
        <button
          type="button"
          onClick={onContinue}
          className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-label-secondary)] transition-colors"
        >
          {t("onboarding.prefs.skip", "Skip — I'll discover as I go")}
        </button>
      </div>
    </div>
  );
}
