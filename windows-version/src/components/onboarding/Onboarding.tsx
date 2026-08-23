import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { type World } from "@/lib/tauri";
import { WelcomeStep } from "./steps/WelcomeStep";
import { PreferenceStep } from "./steps/PreferenceStep";
import { FirstWorldStep } from "./steps/FirstWorldStep";
import { ModelSetupStep } from "./steps/ModelSetupStep";
import { useTranslation } from "react-i18next";

/// First-run flow for a fresh local installation.
///
/// The flow prepares preferences, a first world, a local narrator and an
/// local narrator before entering the main app. Cloud connections stay in
/// Settings so first-run never asks users to make a privacy tradeoff.
type Step = "welcome" | "prefs" | "world" | "engine";

const STEPS: Step[] = ["welcome", "prefs", "world", "engine"];

export function Onboarding() {
  const { t } = useTranslation();
  const setActiveView = useApp((s) => s.setActiveView);
  const setSelectedWorldForDetail = useApp((s) => s.setSelectedWorldForDetail);
  const updatePreference = useApp((s) => s.updatePreference);
  const setGlobalWorlds = useApp((s) => s.setWorlds);

  const [step, setStep] = useState<Step>("welcome");
  const [worlds, setWorlds] = useState<World[] | null>(null);
  const [picked, setPicked] = useState<World | null>(null);

  // The 19 preset worlds are seeded before the window opens (seed.rs, called
  // from lib.rs setup), so this is a read, not a wait. Fetched up front so
  // screen 2 renders instantly when the user gets there.
  useEffect(() => {
    let cancelled = false;
    invoke<World[]>("world_list")
      .then((list) => {
        if (!cancelled) {
          setWorlds(list);
          setGlobalWorlds(list);
        }
      })
      .catch(() => {
        // A failed read must not block the flow — screen 2 degrades to its
        // skip affordance and the user still reaches the app.
        if (!cancelled) setWorlds([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const finish = useCallback(
    async (world: World | null) => {
      // Point the main app at the world they chose, so the last click of
      // onboarding leads somewhere specific instead of a generic library.
      if (world) {
        setSelectedWorldForDetail(world);
        setActiveView("world_detail");
      }
      // Mark onboarding done so App.tsx routes to main app on next render
      updatePreference("onboarding_done", "true");
    },
    [setActiveView, setSelectedWorldForDetail, updatePreference]
  );

  return (
    <div className="h-screen w-screen overflow-y-auto relative">
      {/* FPV landing hero as a subtle backdrop */}
      <div className="fixed inset-0 z-0 pointer-events-none">
        <img
          src="/landing/hero.avif"
          alt=""
          aria-hidden="true"
          className="w-full h-full object-cover opacity-[0.18]"
        />
        <div
          className="absolute inset-0"
          style={{
            background:
              "radial-gradient(circle at 50% 35%, var(--color-bg-elevated) 0%, var(--color-bg-window) 60%, #000 100%)",
            opacity: 0.85,
          }}
        />
      </div>
      <div className="relative z-10">
      <header className="pt-12 pb-1 text-center">
        <h1 className="font-display text-[19px] tracking-[0.04em] text-[var(--color-label-primary)]">
          {t("ui.first-person-viewpoint", "First Person Viewpoint")}
        </h1>
        <p className="mt-2 text-[10.5px] uppercase tracking-[0.15em] text-[var(--color-label-tertiary)]">
          {t("ui.step-inside-the-story", "Step inside the story")}
        </p>
      </header>

      <main className="px-6 pt-6 pb-10">
        <AnimatePresence mode="wait">
          <motion.div
            key={step}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -12 }}
            transition={{ duration: 0.25 }}
          >
            {step === "welcome" && (
              <WelcomeStep
                worldCount={worlds?.length ?? null}
                onContinue={() => setStep("prefs")}
              />
            )}
            {step === "prefs" && (
              <PreferenceStep
                onContinue={() => setStep("world")}
              />
            )}
            {step === "world" && (
              <FirstWorldStep
                worlds={worlds}
                onPick={(w) => {
                  setPicked(w);
                  setStep("engine");
                }}
                onSkip={() => {
                  setPicked(null);
                  setStep("engine");
                }}
                onShowAll={() => {
                  void finish(null);
                  setActiveView("library");
                }}
              />
            )}
            {step === "engine" && (
              <ModelSetupStep
                onComplete={() => finish(picked)}
                busy={false}
                destinationLabel={picked?.name ?? null}
              />
            )}
          </motion.div>
        </AnimatePresence>
      </main>

      <div className="flex justify-center gap-1.5 pb-10">
        {STEPS.map((s) => {
          const idx = STEPS.indexOf(s);
          const curIdx = STEPS.indexOf(step);
          return (
            <button
              key={s}
              onClick={() => { if (idx < curIdx) setStep(s); }}
              disabled={idx >= curIdx}
              aria-label={t("ui.step-number", "Step {{n}}: {{title}}", { n: idx + 1, title: s })}
              className={`w-[5px] h-[5px] rounded-full transition-colors disabled:cursor-default ${
                s === step
                  ? "bg-[var(--color-accent)]"
                  : "bg-[var(--color-label-tertiary)]"
              }`}
            />
          );
        })}
      </div>
      </div>
    </div>
  );
}
