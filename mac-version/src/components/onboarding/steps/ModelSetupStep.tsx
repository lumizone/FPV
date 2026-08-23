import { useEffect, useState } from "react";
import { Download, Check, Cpu, ArrowRight, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Channel, modelList, modelPull, modelSetDefault, type ModelListResult, type PullProgress } from "@/lib/tauri";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

interface Props {
  onComplete: () => void;
  /// Set while the parent is finishing onboarding. Keeps the user from
  /// double-submitting the final step.
  busy?: boolean;
  /// Name of the world chosen on the previous screen, so the final button
  /// names a destination instead of saying "continue".
  destinationLabel?: string | null;
}

/// Final screen of the first run.
///
/// By the time the user arrives, `ensureRequiredModels()` has usually
/// already pulled the RAM-tier chat model in the background — so the default
/// state here is a **confirmation, not a question**. Re-asking the user to
/// choose a model that was already chosen and already downloaded is theatre.
///
/// The picker is the fallback: shown when the pull failed or never ran, or
/// when the user explicitly asks to change engines.
export function ModelSetupStep({ onComplete, busy = false, destinationLabel = null }: Props) {
  const { t } = useTranslation();
  const download = useApp((s) => s.model_download);
  const [data, setData] = useState<ModelListResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [pulling, setPulling] = useState<string | null>(null);
  const [pullProgress, setPullProgress] = useState<{ completed: number; total: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [showPicker, setShowPicker] = useState(false);
  const [continueError, setContinueError] = useState<string | null>(null);

  const refresh = () =>
    modelList()
      .then((result) => {
        setData(result);
        setSelected(result.user_chat_default || result.hardware_chat_default);
      })
      .catch(() => {});

  useEffect(() => {
    refresh().finally(() => setLoading(false));
    // Re-read once the background pull reports done, so a model that landed
    // while the user was picking a world shows as installed here.
  }, [download?.phase]);

  const handleContinue = async () => {
    setContinueError(null);
    if (selected) {
      try {
        await modelSetDefault("chat", selected);
      } catch (error) {
        setContinueError(error instanceof Error ? error.message : String(error));
        return;
      }
    }
    onComplete();
  };

  const handleInstall = async (modelId: string) => {
    setPulling(modelId);
    setPullProgress(null);
    try {
      const ch = new Channel<PullProgress>();
      ch.onmessage = (p) => {
        setPullProgress({ completed: p.completed ?? 0, total: p.total ?? 0 });
      };
      await modelPull(modelId, ch);
      await modelSetDefault("chat", modelId);
      setSelected(modelId);
      await refresh();
    } catch (e) {
      console.error("Pull failed", e);
    } finally {
      setPulling(null);
      setPullProgress(null);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-16">
        <Loader2 className="w-6 h-6 animate-spin text-[var(--color-label-tertiary)]" />
      </div>
    );
  }

  const installed = data?.installed ?? [];
  const catalog = data?.catalog ?? [];
  const chatModels = catalog.filter((m) => m.kind === "chat");
  const installedNames = installed.map((m) => m.name);
  const isInstalled = (id: string) =>
    installedNames.some((n) => n === id || n.startsWith(id + ":"));

  const ready = !!selected && isInstalled(selected);
  const stillPulling = download?.phase === "active";

  // ── Confirmation path ────────────────────────────────────────────
  if (ready && !showPicker) {
    return (
      <div className="max-w-lg mx-auto flex flex-col items-center gap-6 py-6">
        <div className="text-center">
          <h2 className="font-display text-[19px] text-[var(--color-label-primary)] text-balance">
            {t("onboarding.model.ready_title", "Your narrator is ready")}
          </h2>
          <p className="mt-2 font-serif text-[12.5px] text-[var(--color-label-secondary)]">
            {t("onboarding.model.ready_body", {
              defaultValue: "{{model}} — running entirely on this Mac.",
              model: selected,
            })}
          </p>
        </div>

        <div className="flex flex-col items-center gap-2.5 w-full">
          {continueError && <p role="alert" className="text-xs text-red-400">{continueError}</p>}
          <Button
            variant="primary"
            size="lg"
            className="px-7"
            onClick={() => void handleContinue()}
            disabled={busy}
          >
            {busy ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : destinationLabel ? (
              t("onboarding.model.enter_world", {
                defaultValue: "Enter {{world}}",
                world: destinationLabel,
              })
            ) : (
              t("onboarding.model.continue", "Start Writing Stories")
            )}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowPicker(true)}
            disabled={busy}
          >
            {t("onboarding.model.change", "Use a different model")}
          </Button>
        </div>
      </div>
    );
  }

  // ── Picker path (pull failed, never ran, or user asked) ──────────
  return (
    <div className="max-w-lg mx-auto py-2">
      <div className="text-center mb-6">
        <h2 className="font-display text-[19px] text-[var(--color-label-primary)] text-balance">
          {t("onboarding.model.title", "Choose Your Story Engine")}
        </h2>
        <p className="mt-2 font-serif text-[12.5px] text-[var(--color-label-secondary)]">
          {t(
            "onboarding.model.subtitle",
            "Pick a local AI model for generating your narratives. The model runs entirely on your computer — private and offline."
          )}
        </p>
      </div>

      <div className="space-y-2.5 mb-6">
        {chatModels.map((m) => {
          const installedHere = isInstalled(m.id);
          const isSelected = selected === m.id;
          const recommended = m.id === data?.hardware_chat_default;
          return (
            <button
              key={m.id}
              onClick={() => {
                if (!installedHere) handleInstall(m.id);
                else setSelected(m.id);
              }}
              className={cn(
                "w-full flex items-center justify-between p-3.5 rounded-xl border transition-colors text-left",
                "focus-visible:outline-none",
                isSelected
                  ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)]"
                  : "border-[var(--color-separator)] bg-[var(--color-fill-quaternary)] hover:border-[var(--color-label-tertiary)]"
              )}
              disabled={pulling !== null || busy}
            >
              <span className="flex items-center gap-3 min-w-0">
                <Cpu
                  className={cn(
                    "w-5 h-5 shrink-0",
                    isSelected
                      ? "text-[var(--color-accent)]"
                      : "text-[var(--color-label-tertiary)]"
                  )}
                />
                <span className="min-w-0">
                  <span className="flex items-baseline gap-2">
                    <span className="text-[13px] font-medium text-[var(--color-label-primary)] truncate">
                      {m.id}
                    </span>
                    {recommended && (
                      <span className="text-[9px] uppercase tracking-[0.1em] text-[var(--color-accent)] shrink-0">
                        {t("onboarding.model.recommended", "Recommended")}
                      </span>
                    )}
                  </span>
                  {m.description && (
                    <span className="block text-[11px] text-[var(--color-label-tertiary)]">
                      {m.description}
                    </span>
                  )}
                </span>
              </span>
              <span className="shrink-0 pl-3">
                {installedHere ? (
                  isSelected ? (
                    <Check className="w-5 h-5 text-[var(--color-system-green)]" />
                  ) : (
                    <span className="flex items-center gap-2">
                      <Check className="w-4 h-4 text-[var(--color-system-green)]/50" />
                      <ArrowRight className="w-4 h-4 text-[var(--color-label-tertiary)]" />
                    </span>
                  )
                ) : (
                  <span className="flex items-center gap-1.5 text-[11px] text-[var(--color-accent)]">
                    {pulling === m.id ? (
                      <>
                        <Loader2 className="w-4 h-4 animate-spin" />
                        {t("onboarding.model.pulling", "Downloading…")}
                        {pullProgress && pullProgress.total > 0 && (
                          <span className="text-[10px] text-[var(--color-label-tertiary)] ml-1">
                            {Math.round(pullProgress.completed / 1024 / 1024)}MB / {Math.round(pullProgress.total / 1024 / 1024)}MB
                          </span>
                        )}
                      </>
                    ) : (
                      <>
                        <Download className="w-4 h-4" />
                        {t("onboarding.model.install", "Install")}
                      </>
                    )}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>

      <div className="space-y-2.5">
        <Button
          onClick={handleContinue}
          disabled={!ready || busy || pulling !== null}
          variant="primary"
          size="lg"
          className="w-full"
        >
          {busy ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : stillPulling ? (
            t("onboarding.model.waiting", "Still downloading…")
          ) : !ready ? (
            t("onboarding.model.needs_install", "Install a model to continue")
          ) : (
            t("onboarding.model.continue", "Start Writing Stories")
          )}
        </Button>
      </div>
    </div>
  );
}
