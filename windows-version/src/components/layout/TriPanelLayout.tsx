import { useApp } from "@/lib/store";
import { HomeScreen } from "./HomeScreen";
import { WorldLibrary } from "@/components/library/WorldLibrary";
import { WorldCreator } from "@/components/world/WorldCreator";
import { WorldDetail } from "@/components/world/WorldDetail";
import { NarrationScreen } from "@/components/narration/NarrationScreen";
import { LeftPanel } from "./LeftPanel";
import { RightPanel } from "./RightPanel";
import { WindowControls } from "./WindowControls";
import { ResizeHandles } from "./ResizeHandles";
import { startWindowDrag } from "@/lib/tauri";
import { isMac } from "@/lib/platform";
import { AnimatePresence, motion } from "framer-motion";
import { Settings, PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen } from "lucide-react";
import { useTranslation } from "react-i18next";

export function TriPanelLayout() {
  const { t } = useTranslation();
  const activeView = useApp((s) => s.active_view);
  const currentWorldId = useApp((s) => s.current_world_id);
  const worlds = useApp((s) => s.worlds);
  const currentWorld = worlds.find((w) => w.id === currentWorldId);
  const leftOpen = useApp((s) => s.left_panel_open);
  const rightOpen = useApp((s) => s.right_panel_open);
  const setLeftOpen = useApp((s) => s.setLeftPanelOpen);
  const setRightOpen = useApp((s) => s.setRightPanelOpen);

  return (
    <div className="relative flex flex-col h-screen w-screen overflow-hidden text-[var(--color-label-primary)] font-sans">
      <ResizeHandles />
      {/* ── Top bar ───────────────────────────────────────────────── */}
      <div className="fixed top-0 left-0 right-0 h-10 z-[100] flex items-stretch bg-[var(--color-bg-window)]/80 backdrop-blur-md border-b border-[var(--color-separator)]/30">
        {/* Drag region — pl-[78px] clears the macOS traffic-light buttons; Windows has no inset */}
        <div
          data-tauri-drag-region="true"
          onMouseDown={startWindowDrag}
          className={`flex-1 h-full select-none pointer-events-auto flex items-center gap-2 ${isMac ? "pl-[78px]" : "pl-3"}`}
        >
          <img
            src="/app-icon.png"
            alt="FPV"
            className="w-5 h-5 rounded object-contain shrink-0"
          />
          <span className="text-sm font-display tracking-[0.08em] text-[var(--color-accent)] font-semibold">
            FPV
          </span>
          {activeView === "home" && (
             <span className="text-xs text-[var(--color-label-secondary)]">{t("navigation.home", "Home")}</span>
          )}
          {activeView === "session" && currentWorld && (
            <>
              <span className="text-[11px] text-[var(--color-label-quaternary)]">/</span>
              <span className="text-xs text-[var(--color-label-secondary)] truncate max-w-[160px]">
                {currentWorld.name}
              </span>
            </>
          )}
          {activeView === "world_detail" && currentWorld && (
            <>
              <span className="text-[11px] text-[var(--color-label-quaternary)]">/</span>
              <span className="text-xs text-[var(--color-label-secondary)] truncate max-w-[160px]">
                {currentWorld.name}
              </span>
            </>
          )}
          {activeView === "world_new" && (
            <>
              <span className="text-[11px] text-[var(--color-label-quaternary)]">/</span>
              <span className="text-xs text-[var(--color-label-secondary)]">
                 {t("navigation.new_story_short", "New Story")}
              </span>
            </>
          )}
        </div>

        {/* Window controls area — panel toggles + settings */}
        <div className="flex items-center gap-1 pr-2">
          <button
            type="button"
            onClick={() => setLeftOpen(!leftOpen)}
            className="no-drag pointer-events-auto p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
            title={leftOpen ? "Hide left panel" : "Show left panel"}
            aria-label={leftOpen ? "Hide left panel" : "Show left panel"}
          >
            {leftOpen ? <PanelLeftClose className="w-3.5 h-3.5" /> : <PanelLeftOpen className="w-3.5 h-3.5" />}
          </button>
          {activeView === "session" && (
            <button
              type="button"
              onClick={() => setRightOpen(!rightOpen)}
              className="no-drag pointer-events-auto p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
              title={rightOpen ? t("app.layout.hide_right_panel", "Hide right panel") : t("app.layout.show_right_panel", "Show right panel")}
              aria-label={rightOpen ? t("app.layout.hide_right_panel", "Hide right panel") : t("app.layout.show_right_panel", "Show right panel")}
            >
              {rightOpen ? <PanelRightClose className="w-3.5 h-3.5" /> : <PanelRightOpen className="w-3.5 h-3.5" />}
            </button>
          )}
          <button
            type="button"
            onClick={() => useApp.getState().setSettingsOpen(true)}
            className="no-drag pointer-events-auto p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
             title={t("navigation.settings", "Settings")}
             aria-label={t("navigation.settings", "Settings")}
          >
            <Settings className="w-4 h-4" />
          </button>
          {!isMac && <WindowControls />}
        </div>
      </div>

      {/* Spacer for fixed top bar */}
      <div className="h-10 shrink-0" aria-hidden="true" />

      {/* ── Three-panel body ──────────────────────────────────────── */}
      <div className="flex-1 flex min-h-0 overflow-hidden bg-[var(--color-bg-window)]/40">
        {/* Left panel */}
        <motion.aside
          role="navigation"
          aria-label={t("ui.navigation-panel", "Navigation panel")}
          initial={false}
          animate={{ width: leftOpen ? 260 : 0, opacity: leftOpen ? 1 : 0 }}
          transition={{ duration: 0.2, ease: "easeInOut" }}
          className="shrink-0 overflow-hidden border-r border-[var(--color-separator)]/20 bg-[var(--color-bg-window)]"
        >
          <div className="w-[260px] h-full">
            <LeftPanel />
          </div>
        </motion.aside>

        {/* Center panel — always visible */}
        <main className="flex-1 flex flex-col min-w-0 min-h-0 relative bg-[var(--color-bg-content)]">
          <AnimatePresence mode="wait">
            {activeView === "home" && (
              <motion.div
                key="home"
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -12 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className="absolute inset-0"
              >
                <HomeScreen />
              </motion.div>
            )}
            {activeView === "library" && (
              <motion.div
                key="library"
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -12 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className="absolute inset-0"
              >
                <WorldLibrary />
              </motion.div>
            )}
            {activeView === "world_new" && (
              <motion.div
                key="world_new"
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -12 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className="absolute inset-0"
              >
                <WorldCreator />
              </motion.div>
            )}
            {activeView === "world_detail" && (
              <motion.div
                key="world_detail"
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -12 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className="absolute inset-0"
              >
                <WorldDetail />
              </motion.div>
            )}
            {activeView === "session" && (
              <motion.div
                key="session"
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -12 }}
                transition={{ duration: 0.2, ease: "easeInOut" }}
                className="absolute inset-0"
              >
                <NarrationScreen />
              </motion.div>
            )}
          </AnimatePresence>
        </main>

        {/* Right panel — collapsible context */}
        <motion.aside
          role="complementary"
          aria-label={t("ui.context-panel", "Context panel")}
          aria-hidden={!rightOpen}
          inert={!rightOpen}
          initial={false}
          animate={{ width: rightOpen ? 300 : 0, opacity: rightOpen ? 1 : 0 }}
          transition={{ duration: 0.2, ease: "easeInOut" }}
          className="shrink-0 overflow-hidden border-l border-[var(--color-separator)]/20 bg-[var(--color-bg-window)]"
        >
          <div className="w-[300px] h-full">
            <RightPanel />
          </div>
        </motion.aside>
      </div>
    </div>
  );
}
