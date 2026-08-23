import { useEffect, useState, useRef } from "react";
import {
  AlertCircle,
  BookOpen,
  Plus,
  Upload,
} from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import type { World } from "@/lib/tauri";
import { genreGradientCard } from "@/lib/utils";
import { genreAccent } from "@/lib/genreAccents";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

// ── Component ────────────────────────────────────────────────────

/**
 * Browse the locally installed worlds — both the 19 preset worlds
 * seeded at first launch and any user-created ones. Each world card
 * shows a cover image (or genre-swatch placeholder), name, and genre
 * badge. "Start story" creates a session and opens the chat view
 * scoped to that world.
 */
export function WorldLibrary() {
  const { t } = useTranslation();
  const setActiveView = useApp((s) => s.setActiveView);
  const setSelectedWorldForDetail = useApp((s) => s.setSelectedWorldForDetail);
  const worlds = useApp((s) => s.worlds);
  const refreshWorlds = useApp((s) => s.refreshWorlds);
  const [loading, setLoading] = useState(worlds.length === 0);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [importing, setImporting] = useState(false);
  const [genreFilter, setGenreFilter] = useState<string | null>(null);
  const [sourceFilter, setSourceFilter] = useState<"all" | "seed" | "custom">("all");
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Derive unique genres from all worlds for filter chips
  const allGenres = [...new Set(worlds.map((w) => w.genre))]
    .filter(Boolean)
    .sort((a, b) => a.localeCompare(b));

  // Apply genre + source filters
  const visibleWorlds = (() => {
    let filtered = worlds;
    if (sourceFilter === "seed") filtered = filtered.filter((w) => w.source === "seed");
    if (sourceFilter === "custom") filtered = filtered.filter((w) => w.source !== "seed");
    if (genreFilter) filtered = filtered.filter((w) => w.genre === genreFilter);
    return filtered;
  })();

  // ── JSON Import ──────────────────────────────────────────────
  async function handleImportJson(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setImporting(true);
    try {
      const text = await file.text();
      const data = JSON.parse(text);
      // Accept both a single world object or an array
      const items: Array<Record<string, unknown>> = Array.isArray(data) ? data : [data];
      for (const item of items) {
        await invoke("world_import_json", { json: JSON.stringify(item) });
      }
      await refreshWorlds();
    } catch (err: any) {
      console.error("JSON import failed", err);
      setError(err?.message ?? String(err));
    } finally {
      setImporting(false);
      // Reset the input so re-importing the same file works
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  const filteredWorlds = (visibleWorlds.length > 0 || error) && !loading
    ? searchQuery.trim()
      ? visibleWorlds.filter((w) =>
          w.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          (w.description || "").toLowerCase().includes(searchQuery.toLowerCase())
        )
      : visibleWorlds
    : [];

  useEffect(() => {
    if (worlds.length === 0 && !error) {
      setLoading(true);
      refreshWorlds()
        .then(() => setLoading(false))
        .catch((e) => {
          setError((e as { message?: string })?.message ?? String(e));
          setLoading(false);
        });
    } else {
      setLoading(false);
    }
  }, []); // only on mount

  function viewWorldDetail(world: World) {
    setSelectedWorldForDetail(world);
    setActiveView("world_detail");
  }

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
      {/* ── Header ───────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-separator)]/40 shrink-0">
        <h2 className="text-[16px] font-display tracking-[0.04em] flex items-center gap-2">
          <BookOpen className="w-4 h-4 text-[var(--color-accent)]" />
           {t("library.title", "Story Library")}
        </h2>
        <div className="flex items-center gap-3">
          {/* JSON Import */}
          <input
            ref={fileInputRef}
            type="file"
            accept=".json"
            onChange={handleImportJson}
            className="hidden"
          />
          <button
            onClick={() => fileInputRef.current?.click()}
            disabled={importing}
            className="flex items-center gap-1.5 text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors disabled:opacity-50"
             title={t("library.import_json", "Import story from JSON")}
             aria-label={t("library.import_json", "Import story from JSON")}
          >
            <Upload className="w-3.5 h-3.5" />
             {importing ? t("library.importing", "Importing…") : t("library.import", "Import")}
          </button>
          {worlds.length > 0 && (
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
               placeholder={t("library.search", "Search…")}
              className="w-32 text-[12px] px-2.5 py-1.5 rounded-lg bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors"
            />
          )}
          <span className="text-[var(--color-accent)] text-[17px] font-serif italic">
            {loading
               ? t("settings.models.loading", "Loading…")
               : t(filteredWorlds.length === 1 ? "library.story_count_one" : "library.story_count_other", "{{count}} stories", { count: filteredWorlds.length })}
          </span>
        </div>
      </div>

      {/* ── Filter bar: source + genre chips ─────────────────────── */}
      <div className="px-6 py-2.5 border-b border-[var(--color-separator)]/30 shrink-0 space-y-2">
        {/* Source filter */}
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-[9px] uppercase tracking-[0.1em] text-[var(--color-label-quaternary)] font-semibold shrink-0 mr-1">
             {t("library.source", "Source")}
          </span>
          {(["all", "seed", "custom"] as const).map((k) => {
             const label = k === "all" ? t("library.all", "All") : k === "seed" ? t("library.predefined", "Predefined") : t("library.custom", "Custom");
            return (
              <button
                key={k}
                onClick={() => setSourceFilter(k)}
                className={`text-[11px] font-medium px-2.5 py-1 rounded-full border transition-colors ${
                  sourceFilter === k
                    ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                    : "border-[var(--color-separator)] text-[var(--color-label-tertiary)] hover:border-[var(--color-label-tertiary)] hover:text-[var(--color-label-secondary)]"
                }`}
              >
                {label}
              </button>
            );
          })}
        </div>
        {/* Genre filter */}
        {allGenres.length > 0 && (
          <div className="flex items-center gap-1.5 flex-wrap">
            <span className="text-[9px] uppercase tracking-[0.1em] text-[var(--color-label-quaternary)] font-semibold shrink-0 mr-1">
               {t("library.genre", "Genre")}
            </span>
            <button
              onClick={() => setGenreFilter(null)}
              className={`text-[11px] font-medium px-2.5 py-1 rounded-full border transition-colors ${
                !genreFilter
                  ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                  : "border-[var(--color-separator)] text-[var(--color-label-tertiary)] hover:border-[var(--color-label-tertiary)] hover:text-[var(--color-label-secondary)]"
              }`}
            >
               {t("library.all", "All")}
            </button>
            {allGenres.map((g) => {
              const accent = genreAccent(g);
              return (
                <button
                  key={g}
                  onClick={() => setGenreFilter(genreFilter === g ? null : g)}
                  className={`text-[11px] font-medium px-2.5 py-1 rounded-full border transition-colors capitalize`}
                  style={{
                    borderColor: genreFilter === g ? accent : undefined,
                    backgroundColor: genreFilter === g ? `${accent}22` : undefined,
                    color: genreFilter === g ? accent : undefined,
                  }}
                >
                  {g}
                </button>
              );
            })}
            {!genreFilter && (
              <span className="text-[9px] text-[var(--color-label-quaternary)] ml-1">
                 {t("library.click_to_filter", "click to filter")}
              </span>
            )}
          </div>
        )}
      </div>

      {/* ── Scrollable body ──────────────────────────────────── */}
      <div className="flex-1 overflow-y-auto p-6 w-full">
        {error && (
          <div className="bg-red-500/10 border border-red-500/20 rounded-xl p-3 text-[12px] text-[var(--color-system-red)] mb-4">
            <div className="flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
               <span>{t("settings.models.no_data", "Failed to load stories: {{error}}", { error })}</span>
            </div>
          </div>
        )}

        {/* Loading skeleton */}
        {loading && !error && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
            {[1, 2, 3, 4, 5, 6, 7, 8].map((i) => (
              <div
                key={i}
                className="rounded-2xl bg-[var(--color-fill-quaternary)] animate-pulse overflow-hidden"
              >
                <div className="aspect-[2/3] bg-[var(--color-fill-quaternary)]" />
                <div className="p-4 space-y-3">
                  <div className="h-4 w-2/3 bg-[var(--color-fill-quaternary)] rounded" />
                  <div className="h-3 w-1/3 bg-[var(--color-fill-quaternary)] rounded" />
                  <div className="h-8 w-full bg-[var(--color-fill-quaternary)] rounded-lg mt-3" />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* No search results */}
        {worlds.length > 0 && searchQuery && filteredWorlds && filteredWorlds.length === 0 && (
          <div className="text-center py-20">
            <p className="text-[14px] text-[var(--color-label-secondary)]">
               {t("library.no_search_match", "No stories match \"{{query}}\"", { query: searchQuery })}
            </p>
            <p className="text-[12px] text-[var(--color-label-tertiary)] mt-2">
               {t("library.no_search_hint", "Try a different search term or clear the filters.")}
            </p>
            <button
              onClick={() => { setSearchQuery(""); setGenreFilter(null); setSourceFilter("all"); }}
              className="mt-4 text-[12px] text-[var(--color-accent)] hover:underline"
            >
               {t("library.clear_filters", "Clear all filters")}
            </button>
          </div>
        )}

        {/* No results from filter (no search, just genre/source filters) */}
        {worlds.length > 0 && !searchQuery && filteredWorlds.length === 0 && !loading && (
          <div className="text-center py-20 px-6">
            <p className="text-[16px] font-semibold text-[var(--color-label-primary)] font-serif">
               {t("library.no_filter_match", "No stories match the filters")}
            </p>
            <p className="text-[13px] text-[var(--color-label-secondary)] mt-2 max-w-sm mx-auto leading-relaxed">
               {t("library.no_filter_hint", "Clear a filter or create your own story.")}
            </p>
            <div className="flex items-center justify-center gap-3 mt-8">
              <button
                onClick={() => { setGenreFilter(null); setSourceFilter("all"); }}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg border border-[var(--color-separator)] hover:border-[var(--color-accent)]/50 text-[var(--color-label-secondary)] hover:text-[var(--color-accent)] text-[13px] font-medium transition-colors"
              >
                 {t("library.clear_filters", "Clear all filters")}
              </button>
              <button
                onClick={() => { useApp.getState().setEditingWorldId(null); setActiveView("world_new"); }}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] text-black text-[13px] font-semibold transition-colors"
              >
                <Plus className="w-4 h-4" />
                 {t("library.create_own", "Create your own")}
              </button>
            </div>
          </div>
        )}

        {/* Empty state — no worlds installed */}
        {worlds.length === 0 && !error && (
          <div className="text-center py-20 px-6">
            <img
              src="/app-icon.png"
              alt="FPV"
              className="w-20 h-20 rounded-3xl mx-auto mb-6 object-contain"
            />
            <p className="text-[16px] font-semibold text-[var(--color-label-primary)] font-serif">
               {t("library.no_worlds", "Your stories are waiting")}
            </p>
            <p className="text-[13px] text-[var(--color-label-secondary)] mt-2 max-w-sm mx-auto leading-relaxed">
               {t("library.no_worlds_desc", "Create a story, import JSON, or explore the presets to begin.")}
            </p>
            <div className="flex items-center justify-center gap-3 mt-8">
              <button
                onClick={() => fileInputRef.current?.click()}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg border border-[var(--color-separator)] hover:border-[var(--color-accent)]/50 text-[var(--color-label-secondary)] hover:text-[var(--color-accent)] text-[13px] font-medium transition-colors"
              >
                <Upload className="w-4 h-4" />
                 {t("library.import_json", "Import from JSON")}
              </button>
              <button
                onClick={() => { useApp.getState().setEditingWorldId(null); setActiveView("world_new"); }}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] text-black text-[13px] font-semibold transition-colors"
              >
                <Plus className="w-4 h-4" />
                 {t("library.create_own", "Create your own")}
              </button>
            </div>
          </div>
        )}

        {/* World grid */}
        {filteredWorlds.length > 0 && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
            {filteredWorlds.map((world) => (
              <WorldCard
                key={world.id}
                world={world}
                onDetail={() => viewWorldDetail(world)}
              />
            ))}
          </div>
        )}

        {/* "Create your own world" — navigates to the world creator view. */}
        {worlds.length > 0 && !searchQuery && (
          <div className="mt-8 flex justify-center">
            <button
              onClick={() => { useApp.getState().setEditingWorldId(null); setActiveView("world_new"); }}
              className="inline-flex items-center gap-2 px-6 py-3 rounded-xl border-2 border-dashed border-[var(--color-separator)] hover:border-[var(--color-accent)]/50 text-[var(--color-label-secondary)] hover:text-[var(--color-accent)] transition-colors text-[13px] font-medium"
            >
              <Plus className="w-4 h-4" />
               {t("library.create_own", "Create your own")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ── World card ───────────────────────────────────────────────────

function WorldCard({
  world,
  onDetail,
}: {
  world: World;
  onDetail: () => void;
}) {
  const { t } = useTranslation();
  const [imgError, setImgError] = useState(false);
  const hasCover = !!world.cover_image_path && !imgError;
  const accentColor = world.accent_color || genreAccent(world.genre);

  return (
    <button
      type="button"
      onClick={onDetail}
      className="surface rounded-2xl overflow-hidden transition-all duration-200 flex flex-col group hover:-translate-y-1 hover:shadow-xl hover:shadow-black/20 text-left w-full cursor-pointer"
    >
      {/* Cover image */}
      <div className="aspect-[2/3] overflow-hidden bg-[var(--color-fill-quaternary)] relative">
        {hasCover ? (
          <img
            src={convertFileSrc(world.cover_image_path!)}
            alt={world.name}
            className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-105"
            loading="lazy"
            onError={() => setImgError(true)}
          />
        ) : (
          <div
            className={`w-full h-full bg-gradient-to-br ${genreGradientCard(world.genre)} flex flex-col items-center justify-center gap-2`}
          >
            <img
              src="/app-icon.png"
              alt="FPV"
              className="w-14 h-14 object-contain opacity-80"
            />
            <span className="text-[10px] text-[var(--color-label-primary)]/80 font-medium uppercase tracking-widest">
              {world.genre || "custom"}
            </span>
          </div>
        )}
        <span
          className="absolute top-2.5 left-2.5 px-2.5 py-0.5 rounded-full text-[10px] font-medium tracking-wide"
          style={{
            backgroundColor: `${accentColor}22`,
            color: accentColor,
            backdropFilter: "blur(8px)",
            WebkitBackdropFilter: "blur(8px)",
          }}
        >
          {world.genre}
        </span>
        {world.is_nsfw && (
          <span className="absolute top-2.5 right-2.5 px-1.5 py-0.5 rounded-full bg-red-500/70 text-[9px] font-bold text-white tracking-wider">
            18+
          </span>
        )}
        {world.source === "seed" && (
          <span className="absolute bottom-2.5 right-2.5 px-2 py-0.5 rounded-full bg-black/60 backdrop-blur text-[9px] font-semibold text-[var(--color-label-primary)] tracking-wider border border-white/10">
            {t("ui.predefined", "Predefined")}
          </span>
        )}
      </div>

      {/* Content */}
      <div className="p-4 flex flex-col flex-1">
        <h3 className="text-[14px] font-semibold text-[var(--color-label-primary)] leading-tight mb-1 font-serif">
          {world.name}
        </h3>
        {world.description && (
          <p className="text-[12px] text-[var(--color-label-secondary)] leading-relaxed line-clamp-2 mb-3 flex-1">
            {world.description}
          </p>
        )}
        <span className="text-[11px] text-[var(--color-label-tertiary)] mt-auto font-medium">
          {t("ui.view-world", "View world →")}
        </span>
      </div>
    </button>
  );
}
