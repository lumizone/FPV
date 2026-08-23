import { useEffect, useState, lazy, Suspense } from "react";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { SettingsDrawer } from "@/components/settings/SettingsDrawer";
import { TriPanelLayout } from "@/components/layout/TriPanelLayout";
import { Toaster } from "sonner";
import { useApp } from "@/lib/store";
import {
  hardwareDetect,
  settingGetAll,
  settingSet,
} from "@/lib/tauri";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import i18n from "@/i18n/config";

// Only ever rendered once per install, so keep it out of the main bundle.
const Onboarding = lazy(() =>
  import("@/components/onboarding/Onboarding").then((m) => ({
    default: m.Onboarding,
  }))
);

export default function App() {
  const { t } = useTranslation();
  const setHardware = useApp((s) => s.setHardware);
  const setSettingsOpen = useApp((s) => s.setSettingsOpen);
  const setPreferences = useApp((s) => s.setPreferences);
  const setTheme = useApp((s) => s.setTheme);
  const setSelectedModel = useApp((s) => s.setSelectedModel);
  const setActiveView = useApp((s) => s.setActiveView);
  const setLeftPanelOpen = useApp((s) => s.setLeftPanelOpen);
  const setRightPanelOpen = useApp((s) => s.setRightPanelOpen);
  const refreshWorlds = useApp((s) => s.refreshWorlds);
  const preferences = useApp((s) => s.preferences);
  const modelDownload = useApp((s) => s.model_download);
  const [booted, setBooted] = useState(false);
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [bootstrapAttempt, setBootstrapAttempt] = useState(0);

  // Bootstrap
  useEffect(() => {
    let cancelled = false;
    setBooted(false);
    setBootstrapError(null);

    Promise.all([hardwareDetect(), refreshWorlds(), settingGetAll()])
      .then(([hardware, , prefs]) => {
        if (cancelled) return;
        setHardware(hardware);
        setPreferences(prefs);
        if (prefs["language"]) i18n.changeLanguage(prefs["language"]);
        if (prefs["theme"] && ["midnight", "plum", "ocean", "forest", "snow", "sunset", "cyberpunk", "mocha"].includes(prefs["theme"])) setTheme(prefs["theme"] as any);
        if (prefs["user_chat_model"]) setSelectedModel(prefs["user_chat_model"]);
        // Resume the last active story session after a restart — unless
        // onboarding is still pending or the world no longer exists
        // (refreshWorlds ran above as part of this bootstrap).
        if (
          prefs["onboarding_done"] === "true" &&
          prefs["last_session_id"] &&
          prefs["last_world_id"] &&
          useApp.getState().worlds.some((w) => w.id === prefs["last_world_id"])
        ) {
          useApp.getState().setActiveSession(prefs["last_session_id"], prefs["last_world_id"]);
          setActiveView("session");
        }
        // Restore panel open/closed state persisted by the ⌘⇧R / ⌘\ toggles.
        if (prefs["right_panel_open"] === "false") setRightPanelOpen(false);
        if (prefs["left_panel_open"] === "false") setLeftPanelOpen(false);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setBootstrapError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (!cancelled) setBooted(true);
      });

    return () => {
      cancelled = true;
    };
  }, [bootstrapAttempt, refreshWorlds, setHardware, setPreferences, setSelectedModel, setTheme]);

  // Keyboard shortcuts
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      const shift = e.shiftKey;
      // Cmd/Ctrl+, → Settings
      if (mod && !shift && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
      }
      // Cmd/Ctrl+Shift+N → New world
      if (mod && shift && e.key === "N") {
        e.preventDefault();
        useApp.getState().setEditingWorldId(null);
        setActiveView("world_new");
      }
      // Cmd/Ctrl+Shift+K → Home
      if (mod && shift && e.key === "K") {
        e.preventDefault();
        setActiveView("home");
      }
      // Cmd/Ctrl+Shift+R → Toggle right panel (only during story)
      if (mod && shift && e.key === "R") {
        const store = useApp.getState();
        if (store.active_view !== "session") return;
        e.preventDefault();
        const next = !store.right_panel_open;
        store.setRightPanelOpen(next);
        settingSet("right_panel_open", String(next)).catch(() => {});
      }
      // Cmd/Ctrl+\ → Toggle left panel
      if (mod && !shift && e.key === "\\") {
        e.preventDefault();
        const store = useApp.getState();
        const next = !store.left_panel_open;
        store.setLeftPanelOpen(next);
        settingSet("left_panel_open", String(next)).catch(() => {});
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSettingsOpen, setActiveView]);

  // Theme tokens — sets data-theme attribute for CSS-driven theming
  const theme = useApp((s) => s.theme);
  useEffect(() => {
    document.body.setAttribute("data-theme", theme);
  }, [theme]);

  // External links
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      const anchor = target?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!anchor) return;
      const href = anchor.getAttribute("href") || "";
      if (/^https?:\/\//i.test(href)) {
        e.preventDefault();
        openUrl(href).catch(() => {});
      }
    };
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, []);

  // Blur-on-background
  const blurOnBackground = useApp(
    (s) => s.preferences["blurOnBackground"] === "true"
  );
  useEffect(() => {
    if (!blurOnBackground) {
      document.body.style.filter = "";
      return;
    }
    const onHide = () => {
      document.body.style.filter = "blur(12px)";
    };
    const onShow = () => {
      document.body.style.filter = "";
    };
    const onVisibility = () => {
      document.hidden ? onHide() : onShow();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", onHide);
    window.addEventListener("focus", onShow);
    return () => {
      document.body.style.filter = "";
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", onHide);
      window.removeEventListener("focus", onShow);
    };
  }, [blurOnBackground]);

  // Routing: boot → onboarding (first run) → main app
  if (!booted) {
    return (
      <ErrorBoundary>
        <BootScreen />
      </ErrorBoundary>
    );
  }
  if (bootstrapError) {
    return (
      <ErrorBoundary>
        <BootError
          message={bootstrapError}
          onRetry={() => setBootstrapAttempt((attempt) => attempt + 1)}
          t={t}
        />
      </ErrorBoundary>
    );
  }
  // First run: show onboarding, then flip to main app on completion
  const onboardingDone = preferences["onboarding_done"] === "true";
  if (!onboardingDone) {
    return (
      <ErrorBoundary>
        <Suspense fallback={<BootScreen />}>
          <Onboarding />
        </Suspense>
      </ErrorBoundary>
    );
  }

  // Main app
  return (
    <ErrorBoundary>
      <TriPanelLayout />
      <SettingsDrawer />
      <Toaster
        theme={theme === "snow" ? "light" : "dark"}
        position="top-right"
        closeButton
        toastOptions={{
          className:
            "bg-[var(--color-bg-elevated)]/90 backdrop-blur-xl border-[var(--color-separator)] text-[var(--color-label-primary)]",
          descriptionClassName: "text-[var(--color-label-secondary)]",
        }}
      />
      {modelDownload?.phase === "active" &&
        (() => {
          const pct =
            modelDownload.total > 0
              ? Math.min(
                  100,
                  Math.round(
                    (modelDownload.completed / modelDownload.total) * 100
                  )
                )
              : null;
          const cancelDownload = useApp.getState().cancelModelDownload;
          return (
            <div className="fixed bottom-4 right-4 z-[60] w-64 surface-elevated px-4 py-2.5 shadow-2xl">
              <div className="flex items-center gap-2 text-[12px] font-medium text-[var(--color-label-primary)] mb-1.5">
                <span className="h-2 w-2 rounded-full bg-[var(--color-accent)] animate-pulse shrink-0" />
                <span className="truncate">
                  {t("app.modeldl.downloading", "Downloading {{model}}", {
                    model: modelDownload.id,
                  })}
                </span>
              </div>
              <div className="text-[10px] text-[var(--color-label-secondary)] mb-1">
                {modelDownload.status}
                {pct !== null ? ` · ${pct}%` : ""}
              </div>
              <div className="h-1 bg-[var(--color-fill-primary)] rounded-full overflow-hidden mb-2">
                <div
                  className={`h-full bg-[var(--color-accent)] transition-[width] duration-300 ${
                    pct === null ? "animate-pulse w-1/3" : ""
                  }`}
                  style={pct !== null ? { width: `${pct}%` } : undefined}
                />
              </div>
              <button
                onClick={() => cancelDownload()}
                className="w-full text-[10px] text-red-400 hover:text-red-300 transition-colors text-center"
              >
                Cancel download
              </button>
            </div>
          );
        })()}
    </ErrorBoundary>
  );
}

function BootScreen() {
  return (
    <div
      className="flex flex-col items-center justify-center h-screen w-screen"
      style={{
        background:
          "radial-gradient(circle at 50% 35%, var(--color-bg-elevated) 0%, var(--color-bg-window) 60%, #000 100%)",
      }}
    >
      <div className="font-display text-[18px] leading-relaxed text-[var(--color-label-primary)] max-w-[220px] text-center tracking-[0.04em]">
        First Person Viewpoint
      </div>
      <div className="mt-3 text-[18px] text-[var(--color-warm)] font-serif italic">
        ✦
      </div>
      <div className="mt-6 text-[11px] text-[var(--color-label-tertiary)] tracking-[0.15em] uppercase">
        Step inside the story
      </div>
    </div>
  );
}

function BootError({
  message,
  onRetry,
  t,
}: {
  message: string;
  onRetry: () => void;
  t: (key: string, fallback: string) => string;
}) {
  return (
    <div
      className="flex flex-col items-center justify-center h-screen w-screen px-6 text-center"
      style={{
        background:
          "radial-gradient(circle at 50% 35%, var(--color-bg-elevated) 0%, var(--color-bg-window) 60%, #000 100%)",
      }}
    >
      <div className="font-display text-[18px] text-[var(--color-label-primary)]">
        {t("errors.title", "Something went wrong")}
      </div>
      <p className="mt-3 max-w-md text-[13px] leading-relaxed text-[var(--color-label-secondary)]">
        {t("errors.body", "A piece of the interface failed. Your stories and worlds are safe.")}
      </p>
      <p className="mt-3 max-w-md text-[11px] text-[var(--color-label-tertiary)] break-words">
        {message}
      </p>
      <button
        type="button"
        onClick={onRetry}
        className="mt-6 rounded-lg bg-[var(--color-accent)] px-5 py-2.5 text-[12px] font-semibold text-black transition-colors hover:brightness-110"
      >
        {t("errors.retry", "Try again")}
      </button>
    </div>
  );
}
