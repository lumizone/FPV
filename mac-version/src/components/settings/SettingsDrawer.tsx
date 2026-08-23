import {
  Suspense,
  lazy,
  Component,
  useEffect,
  useRef,
  type ReactNode,
  type ErrorInfo,
} from "react";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import {
  Settings as SettingsIcon,
  Palette,
  Cpu,
  Image,
  Cloud,
  Info,
  Loader2,
  AlertTriangle,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import i18n from "@/i18n/config";

/// Local error boundary so a crash in ONE settings tab (a lazy chunk
/// load failure after an app update, an unexpected null shape, etc.)
/// shows an inline error instead of propagating to the root
/// ErrorBoundary and crashing the whole window. Mirrors Local Waifu's
/// SettingsTabBoundary.
class SettingsTabBoundary extends Component<
  { tabId: string; children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("SettingsTab crashed:", this.props.tabId, error, info);
  }
  componentDidUpdate(prev: { tabId: string }) {
    // Re-mount the boundary state on tab switch so the user can
    // recover by clicking another tab and back.
    if (prev.tabId !== this.props.tabId && this.state.error) {
      this.setState({ error: null });
    }
  }
  render() {
    const t = i18n.t;
    if (this.state.error) {
      return (
        <div className="flex flex-col items-center justify-center py-16 text-center gap-3">
          <AlertTriangle className="w-8 h-8 text-[var(--color-label-tertiary)]" />
          <div className="text-[15px] font-medium text-[var(--color-label-primary)]">
            {t("ui.this-tab-failed-to-load", "This tab failed to load.")}
          </div>
          <div className="text-[13px] text-[var(--color-label-tertiary)] max-w-md">
            {this.state.error.message || "Unknown error"}
          </div>
          <button
            className="text-[13px] text-[var(--color-system-blue)] underline mt-2"
            onClick={() => this.setState({ error: null })}
          >
            {t("ui.try-again", "Try again")}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

// Settings tabs are lazy-loaded so the initial app bundle doesn't pay
// for code paths the user only hits when they explicitly open Settings
// (Cmd+,). Each tab arrives as its own chunk on demand.
const GeneralTab = lazy(() =>
  import("./GeneralTab").then((m) => ({ default: m.GeneralTab }))
);
const AppearanceTab = lazy(() =>
  import("./AppearanceTab").then((m) => ({ default: m.AppearanceTab }))
);
const NarrativeModelTab = lazy(() =>
  import("./NarrativeModelTab").then((m) => ({ default: m.NarrativeModelTab }))
);
const ImageModelTab = lazy(() =>
  import("./ImageModelTab").then((m) => ({ default: m.ImageModelTab }))
);
const ByokTab = lazy(() =>
  import("./ByokTab").then((m) => ({ default: m.ByokTab }))
);
const AboutTab = lazy(() =>
  import("./AboutTab").then((m) => ({ default: m.AboutTab }))
);

export function SettingsDrawer() {
  const { t } = useTranslation();
  const open = useApp((s) => s.settings_open);
  const tab = useApp((s) => s.settings_tab);
  const setTab = useApp((s) => s.setSettingsTab);
  const setOpen = useApp((s) => s.setSettingsOpen);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const focusFirst = window.setTimeout(() => {
      dialogRef.current?.querySelector<HTMLElement>("button, [href], input, select, textarea")?.focus();
    }, 0);
    return () => {
      window.clearTimeout(focusFirst);
      previous?.focus?.();
    };
  }, [open]);

  // Narrative and Image models are deliberately SEPARATE sidebar tabs —
  // the user asked to keep them apart even though the Local Waifu design
  // folds them into one segmented control.
  const tabs = [
    { id: "general", label: t("settings.drawer.general", "General"), icon: SettingsIcon },
    { id: "appearance", label: t("settings.drawer.appearance", "Appearance"), icon: Palette },
    { id: "narrative_model", label: t("settings.drawer.narrative_model", "Narrative Model"), icon: Cpu },
    { id: "image_model", label: t("settings.drawer.image_model", "Image Model"), icon: Image },
    { id: "byok", label: t("settings.drawer.byok", "AI Cloud Connection"), icon: Cloud },
    { id: "about", label: t("settings.drawer.about", "About"), icon: Info },
  ] as const;

  if (!open) return null;

  function renderContent() {
    switch (tab) {
      case "general": return <GeneralTab />;
      case "appearance": return <AppearanceTab />;
      case "narrative_model": return <NarrativeModelTab />;
      case "image_model": return <ImageModelTab />;
      case "byok": return <ByokTab />;
      case "about": return <AboutTab />;
      default: return null;
    }
  }

  const activeLabel = tabs.find((t_) => t_.id === tab)?.label ?? "Settings";

  return (
    <div
      role="presentation"
      className="fixed inset-0 z-[200] flex items-center justify-center bg-black/40 backdrop-blur-md"
      onClick={(e) => {
        if (e.target === e.currentTarget) setOpen(false);
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-dialog-title"
        aria-describedby="settings-dialog-description"
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            setOpen(false);
            return;
          }
          if (event.key !== "Tab" || !dialogRef.current) return;
          const focusable = Array.from(dialogRef.current.querySelectorAll<HTMLElement>("button, [href], input, select, textarea, [tabindex]:not([tabindex=\"-1\"])"))
            .filter((element) => !element.hasAttribute("disabled"));
          if (focusable.length === 0) return;
          const first = focusable[0];
          const last = focusable[focusable.length - 1];
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
          }
        }}
        className="max-w-[1000px] w-[90vw] h-[85vh] p-0 overflow-hidden flex flex-col bg-[var(--color-bg-window)]/85 backdrop-blur-3xl border border-white/10 rounded-2xl shadow-2xl"
      >
        {/* Header */}
        <div className="h-14 shrink-0 border-b border-white/10 flex items-center justify-between px-6">
          <div>
          <div id="settings-dialog-title" className="text-[15px] font-semibold text-[var(--color-label-primary)] tracking-wide">
            {t("settings.drawer.title", "Settings")}
          </div>
          <p id="settings-dialog-description" className="sr-only">{t("ui.application-settings-and-local-ai-configuration", "Application settings and local AI configuration")}</p>
          </div>
          <button onClick={() => setOpen(false)} aria-label={t("ui.close-settings", "Close settings")} className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)]">
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 flex min-h-0">
          {/* Left Sidebar Menu */}
          <div className="w-[240px] shrink-0 border-r border-white/10 p-3 overflow-y-auto space-y-1">
            {tabs.map((t_) => {
              const Icon = t_.icon;
              const isActive = tab === t_.id;
              return (
                <button
                  key={t_.id}
                  onClick={() => setTab(t_.id)}
                  className={cn(
                    "w-full flex items-center gap-3 px-3 py-2 rounded-xl text-[13px] font-medium transition-all text-left",
                    isActive
                      ? "bg-[var(--color-accent)] text-black shadow-sm"
                      : "text-[var(--color-label-secondary)] hover:bg-black/10 hover:text-[var(--color-label-primary)]"
                  )}
                >
                  <Icon className="w-[18px] h-[18px] shrink-0" strokeWidth={1.75} />
                  <span>{t_.label}</span>
                </button>
              );
            })}
          </div>

          {/* Right Content Area */}
          <div className="flex-1 overflow-y-auto p-8 relative">
            <div className="max-w-2xl mx-auto pb-12">
              <div className="mb-8">
                <h2 className="text-[20px] font-semibold text-[var(--color-label-primary)] tracking-tight">
                  {activeLabel}
                </h2>
              </div>
              <SettingsTabBoundary tabId={tab}>
                <Suspense
                  fallback={
                    <div className="flex items-center justify-center py-12 text-[var(--color-label-tertiary)]">
                      <Loader2 className="w-5 h-5 animate-spin" />
                    </div>
                  }
                >
                  {renderContent()}
                </Suspense>
              </SettingsTabBoundary>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
