import { convertFileSrc } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { Plus, Compass, Library, Play, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { World } from "@/lib/tauri";

/// First screen after onboarding — a welcoming home with three clear
/// paths: create, browse, or check your played stories.
export function HomeScreen() {
  const { t } = useTranslation();
  const worlds = useApp((s) => s.worlds);
  const setActiveView = useApp((s) => s.setActiveView);
  const setLeftPanelOpen = useApp((s) => s.setLeftPanelOpen);
  const setSelectedWorldForDetail = useApp((s) => s.setSelectedWorldForDetail);

  // Lead with a deliberately varied, approachable shelf rather than database
  // order. Genre fallbacks preserve a useful home screen for imported seeds.
  const starterTitles = ["Ashwick County", "Dark Village", "Neon Tokyo", "The Last Train", "Moonlit Garden", "Steel and Silk"];
  const featured = [
    ...starterTitles.map((name) => worlds.find((world) => world.name === name)).filter((world): world is World => Boolean(world)),
    ...worlds.filter((world) => world.source === "seed"),
  ].filter((world, index, list) => list.findIndex((item) => item?.id === world?.id) === index).slice(0, 6);

  return (
    <div className="flex-1 flex flex-col h-full overflow-y-auto bg-[var(--color-bg-content)]">
      {/* Hero area */}
      <div className="flex flex-col items-center justify-center py-12 px-6 text-center">
        <div className="mb-3 flex h-11 w-11 items-center justify-center rounded-2xl bg-[var(--color-accent-soft)] text-[var(--color-accent)]"><Sparkles className="h-5 w-5" /></div>
        <h1 className="text-[28px] font-display tracking-[0.04em] text-[var(--color-label-primary)]">
          {t("ui.first-person-viewpoint", "First Person Viewpoint")}
        </h1>
        <p className="mt-2 text-[14px] text-[var(--color-label-secondary)] font-serif max-w-md leading-relaxed">
          {t("onboarding.welcome.body_generic", "Choose a world, make a move, and keep every consequence. Stories run locally by default.")}
        </p>
      </div>

      {/* Quick actions */}
      <div className="px-6 pb-8 max-w-sm mx-auto w-full space-y-3">
        <button
          onClick={() => { useApp.getState().setEditingWorldId(null); setActiveView("world_new"); }}
          className="w-full flex items-center gap-3 px-5 py-4 rounded-xl bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_85%,white)] text-black text-[14px] font-semibold transition-colors shadow-lg shadow-[var(--color-accent)]/20"
        >
          <Plus className="w-5 h-5" />
           {t("navigation.new_story", "Create your own world")}
        </button>

        <button
          onClick={() => {
            setActiveView("library");
          }}
          className="w-full flex items-center gap-3 px-5 py-4 rounded-xl border border-[var(--color-separator)] hover:border-[var(--color-accent)]/30 bg-[var(--color-fill-quaternary)]/50 text-[var(--color-label-primary)] text-[14px] font-medium transition-colors"
        >
          <Compass className="w-5 h-5 text-[var(--color-accent)]" />
           {t("navigation.explore", "Browse all worlds")}
        </button>

        <button
          onClick={() => {
            // Resume the most recent adventure when one exists; otherwise
            // open the left panel so recent sessions are visible.
            const prefs = useApp.getState().preferences;
            if (
              prefs["last_session_id"] &&
              prefs["last_world_id"] &&
              useApp.getState().worlds.some((w) => w.id === prefs["last_world_id"])
            ) {
              useApp.getState().setActiveSession(prefs["last_session_id"], prefs["last_world_id"]);
              setActiveView("session");
            } else {
              setLeftPanelOpen(true);
            }
          }}
          className="w-full flex items-center gap-3 px-5 py-4 rounded-xl border border-[var(--color-separator)] hover:border-[var(--color-accent)]/30 bg-[var(--color-fill-quaternary)]/50 text-[var(--color-label-secondary)] text-[14px] font-medium transition-colors"
        >
          <Library className="w-5 h-5" />
           {t("navigation.continue", "Continue an adventure")}
        </button>
      </div>

      {/* Featured stories */}
      {featured.length > 0 && (
        <div className="px-6 pb-10">
          <p className="text-[11px] uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] font-semibold mb-3 text-center">
              {t("navigation.featured", "Start here")}
          </p>
           <p className="mb-4 text-center font-serif text-[12px] text-[var(--color-label-tertiary)]">{t("navigation.featured_desc", "A hand-picked first shelf across moods and genres.")}</p>
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-3 max-w-xl mx-auto">
            {featured.map((w) => (
              <button
                key={w.id}
                onClick={() => {
                  setSelectedWorldForDetail(w);
                  setActiveView("world_detail");
                }}
                className="group text-left rounded-xl overflow-hidden border border-[var(--color-separator)]/30 bg-[var(--color-fill-quaternary)] hover:border-[var(--color-accent)]/30 transition-colors"
              >
                {/* Cover thumbnail */}
                <div className="aspect-[2/3] bg-[var(--color-fill-tertiary)] flex items-center justify-center relative overflow-hidden">
                  {w.cover_image_path ? (
                    <img
                      src={convertFileSrc(w.cover_image_path)}
                      alt=""
                      className="w-full h-full object-cover opacity-40 group-hover:opacity-60 transition-opacity"
                      onError={(e) => {
                        (e.target as HTMLImageElement).style.display = "none";
                      }}
                    />
                  ) : null}
                  <div className="absolute inset-0 flex items-center justify-center">
                    <Play className="w-6 h-6 text-[var(--color-label-primary)]/40 group-hover:text-[var(--color-accent)] group-hover:opacity-100 transition-all" />
                  </div>
                </div>
                <div className="p-2.5">
                  <p className="text-[11px] font-medium text-[var(--color-label-primary)] truncate">
                    {w.name}
                  </p>
                  <p className="text-[9px] uppercase tracking-wider text-[var(--color-label-tertiary)]">
                    {w.genre}
                  </p>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
