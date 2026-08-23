import { Button } from "@/components/ui/button";
import { useTranslation } from "react-i18next";

interface Props {
  onContinue: () => void;
  /// How many worlds were seeded. Passed in rather than hardcoded so the
  /// copy stays true if the seed file grows.
  worldCount: number | null;
}

/// First screen of the first run.
///
/// Deliberately holds one paragraph and one button. The model setup step
/// handles downloads and hardware-specific choices later in the flow.
///
/// The app is free; optional BYOK cloud providers are configured separately.
export function WelcomeStep({ onContinue, worldCount }: Props) {
  const { t } = useTranslation();

  return (
    <div className="max-w-lg mx-auto flex flex-col items-center gap-7 py-6">
      <div className="w-40 h-40 rounded-3xl overflow-hidden border border-[var(--color-separator)]/30 shadow-2xl shadow-black/40">
        <img
          src="/onboarding/intro-hero.webp"
          alt="First Person Viewpoint"
          className="w-full h-full object-cover"
        />
      </div>
      <p className="font-serif text-[14px] leading-[1.7] text-[var(--color-label-secondary)] text-center text-balance">
        {worldCount
          ? t("onboarding.welcome.body", {
              defaultValue:
                "{{count}} worlds are waiting. Stories run locally by default — no account required, and no cloud unless you connect your own key.",
              count: worldCount,
            })
          : t("onboarding.welcome.body_generic", {
              defaultValue:
                "Stories run locally by default — no account required, and no cloud unless you connect your own key.",
            })}
      </p>

      <Button variant="primary" size="lg" className="px-8" onClick={onContinue}>
        {t("onboarding.welcome.begin", "Begin")}
      </Button>

      <ul className="flex flex-wrap justify-center gap-x-5 gap-y-1.5 list-none p-0 m-0">
        {[
          t("onboarding.welcome.trust_local", "Runs on your Mac"),
          t("onboarding.welcome.trust_account", "No account"),
           t("onboarding.welcome.trust_private", "Local by default"),
        ].map((line) => (
          <li
            key={line}
            className="text-[10.5px] tracking-[0.04em] text-[var(--color-label-tertiary)]"
          >
            {line}
          </li>
        ))}
      </ul>
    </div>
  );
}
