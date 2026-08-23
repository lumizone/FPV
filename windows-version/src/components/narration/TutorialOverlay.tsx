import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useApp } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { useTranslation } from "react-i18next";
import { PenLine, PanelLeft, BookOpen } from "lucide-react";

/**
 * TutorialOverlay — shown once on first entry to NarrationScreen.
 *
 * Three positioned tooltips that explain the three-panel layout
 * and input mechanics. Dismissed with "Got it", stored as
 * `tutorial_shown` preference so it never fires again.
 */
export function TutorialOverlay() {
  const { t } = useTranslation();
  const updatePreference = useApp((s) => s.updatePreference);
  const [step, setStep] = useState(0);
  const [visible, setVisible] = useState(true);

  function dismiss() {
    updatePreference("tutorial_shown", "true");
    setVisible(false);
  }

  const tips = [
    {
      title: t("tutorial.input_title", "Your story begins here"),
      body: t(
        "tutorial.input_body",
        "Write what your character does, says, or how the scene unfolds. Switch between Do, Say, and Story modes to shape the narrative."
      ),
      icon: <PenLine className="w-4 h-4" />,
      position: "bottom",
      className: "bottom-24 left-1/2 -translate-x-1/2",
    },
    {
      title: t("tutorial.left_title", "Your worlds & chronicles"),
      body: t(
        "tutorial.left_body",
        "The left panel holds your worlds, sessions, and quick actions. Switch between stories without losing your place."
      ),
      icon: <PanelLeft className="w-4 h-4" />,
      position: "top-left",
      className: "top-20 left-[270px]",
    },
    {
      title: t("tutorial.right_title", "Lore at a glance"),
      body: t(
        "tutorial.right_body",
        "The right panel shows your world's codex, characters, and what the narrator knows. Toggle it anytime with the panel buttons above."
      ),
      icon: <BookOpen className="w-4 h-4" />,
      position: "top-right",
      className: "top-20 right-[310px]",
    },
  ];

  const current = tips[step];

  return (
    <AnimatePresence>
      {visible && (
        <>
          {/* Backdrop */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-[150] bg-black/40 backdrop-blur-[2px]"
            onClick={dismiss}
          />

          {/* Tooltip card */}
          <motion.div
            key={step}
            initial={{ opacity: 0, y: 8, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.97 }}
            transition={{ duration: 0.2 }}
            className={`fixed z-[151] w-[280px] ${current.className}`}
          >
            <div className="surface-elevated p-4 border-[var(--color-accent)]/20 shadow-2xl">
              <div className="flex items-start gap-3">
                <div className="w-8 h-8 rounded-lg bg-[var(--color-accent-soft)] flex items-center justify-center text-[var(--color-accent)] shrink-0">
                  {current.icon}
                </div>
                <div className="flex-1 min-w-0">
                  <h4 className="text-[13px] font-display tracking-[0.02em] text-[var(--color-label-primary)] mb-1">
                    {current.title}
                  </h4>
                  <p className="text-[11px] leading-relaxed text-[var(--color-label-secondary)]">
                    {current.body}
                  </p>
                </div>
              </div>

              {/* Step dots + actions */}
              <div className="flex items-center justify-between mt-4 pt-3 border-t border-[var(--color-separator)]/20">
                <div className="flex gap-1">
                  {tips.map((_, i) => (
                    <div
                      key={i}
                      className={`w-1.5 h-1.5 rounded-full transition-colors ${
                        i === step
                          ? "bg-[var(--color-accent)]"
                          : "bg-[var(--color-label-quaternary)]"
                      }`}
                    />
                  ))}
                </div>
                <div className="flex items-center gap-2">
                  {step > 0 && (
                    <button
                      type="button"
                      onClick={() => setStep(step - 1)}
                      className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors"
                    >
                      {t("tutorial.back", "Back")}
                    </button>
                  )}
                  {step < tips.length - 1 ? (
                    <Button variant="primary" size="sm" onClick={() => setStep(step + 1)}>
                      {t("tutorial.next", "Next")}
                    </Button>
                  ) : (
                    <Button variant="primary" size="sm" onClick={dismiss}>
                      {t("tutorial.got_it", "Got it")}
                    </Button>
                  )}
                </div>
              </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
