import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import {
  Plus,
  Play,
  Library,
  MessageSquare,
  Trash2,
  Home,
} from "lucide-react";
import type { SessionSummary } from "@/lib/tauri";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

/**
 * LeftPanel — your played adventures.
 *
 * ONLY shows stories you've actually started (have at least one session).
 * Grouped by story, with sessions below. "Browse all stories" opens the
 * full library in the center panel. Delete button per adventure.
 */
export function LeftPanel() {
  const { t } = useTranslation();
  const worlds = useApp((s) => s.worlds);
  const currentWorldId = useApp((s) => s.current_world_id);
  const currentSessionId = useApp((s) => s.current_session_id);
  const setActiveView = useApp((s) => s.setActiveView);
  const setActiveSession = useApp((s) => s.setActiveSession);
  const setSelectedWorldForDetail = useApp((s) => s.setSelectedWorldForDetail);
  const refreshWorlds = useApp((s) => s.refreshWorlds);
  const clearRuntimeState = useApp((s) => s.clearRuntimeState);
  const selectedModel = useApp((s) => s.selected_model);

  const [allSessions, setAllSessions] = useState<
    (SessionSummary & { world_name: string; world_genre: string })[]
  >([]);
  const [sessionsLoading, setSessionsLoading] = useState(true);

  const loadSessions = useCallback(async () => {
    const out: typeof allSessions = [];
    for (const w of worlds) {
      try {
        const list = await invoke<SessionSummary[]>(
          "session_list_for_world",
          { world_id: w.id }
        );
        for (const s of list.filter((session) => !session.is_checkpoint)) {
          out.push({ ...s, world_name: w.name, world_genre: w.genre });
        }
      } catch {
        // skip
      }
    }
    out.sort(
      (a, b) =>
        new Date(b.updated_at || 0).getTime() -
        new Date(a.updated_at || 0).getTime()
    );
    setAllSessions(out);
    setSessionsLoading(false);
  }, [worlds]);

  useEffect(() => {
    setSessionsLoading(true);
    void loadSessions();
  }, [loadSessions, currentSessionId]);

  // Only worlds that have at least one session
  const playedWorldIds = new Set(allSessions.map((s) => s.world_id));
  const playedWorlds = worlds.filter((w) => playedWorldIds.has(w.id));

  async function startNew(worldId: string) {
    try {
      const sessionId = await invoke<string>("session_create", { world_id: worldId });
      setActiveSession(sessionId, worldId);
      setActiveView("session");
    } catch (e) {
      console.error("Failed to start:", e);
    }
  }

  function continueAdventure(session: SessionSummary) {
    setActiveSession(session.id, session.world_id);
    setActiveView("session");
  }

  function goHome() {
    setActiveView("home");
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="px-3 py-3 border-b border-[var(--color-separator)]/20">
        <button
          onClick={goHome}
          className="flex items-center gap-2 text-[12px] font-semibold text-[var(--color-label-primary)] hover:text-[var(--color-accent)] transition-colors w-full"
        >
          <Home className="w-4 h-4" />
           {t("navigation.home", "Home")}
        </button>
      </div>

      {/* Adventures list */}
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {sessionsLoading ? (
          <div className="flex items-center justify-center py-8">
            <div className="w-4 h-4 rounded-full border-2 border-[var(--color-accent)] border-t-transparent animate-spin" />
          </div>
        ) : playedWorlds.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center px-2">
            <MessageSquare className="w-6 h-6 text-[var(--color-label-quaternary)] mb-2" />
            <p className="text-[11px] text-[var(--color-label-tertiary)] leading-relaxed mb-3">
               {t("navigation.no_adventures", "No adventures yet. Pick a story to begin your first.")}
            </p>
            <button
              onClick={() => setActiveView("library")}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-[var(--color-accent)] text-black text-[11px] font-semibold"
            >
              <Library className="w-3.5 h-3.5" />
               {t("navigation.browse_stories", "Browse all stories")}
            </button>
          </div>
        ) : (
          <div className="space-y-4">
            <div className="flex items-center justify-between">
              <span className="text-[10px] uppercase tracking-[0.12em] text-[var(--color-label-tertiary)] font-semibold">
                 {t("navigation.adventures", "Your Adventures")}
              </span>
              <span className="text-[9px] text-[var(--color-label-quaternary)]">
                {allSessions.length}
              </span>
            </div>

            {playedWorlds.map((w) => {
              const worldSessions = allSessions.filter(
                (s) => s.world_id === w.id
              );
              const isActiveWorld = currentWorldId === w.id && !!currentSessionId;
              return (
                <div key={w.id} className="space-y-0.5">
                  {/* Story name + delete */}
                  <div className="flex items-center gap-1 group">
                    <button
                      onClick={() => {
                        setSelectedWorldForDetail(w);
                        setActiveView("world_detail");
                      }}
                      className={`flex-1 flex items-center gap-2 px-2 py-1 rounded-md text-[12px] transition-colors text-left ${
                        isActiveWorld
                          ? "text-[var(--color-accent)] font-medium"
                          : "text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)]"
                      }`}
                    >
                      <span
                        className="w-2 h-2 rounded-full shrink-0"
                        style={{
                          backgroundColor: w.accent_color || "var(--color-accent)",
                        }}
                      />
                      <span className="truncate">{w.name}</span>
                    </button>
                    <button
                      onClick={() => {
                        if (window.confirm(`Delete "${w.name}" and all its adventures?`)) {
                          invoke("world_delete", { id: w.id })
                            .then(async () => {
                              if (currentWorldId === w.id) clearRuntimeState();
                              await refreshWorlds();
                            })
                            .catch((error) => toast.error(t("ui.failed-to-delete-world", "Failed to delete world."), { description: String(error) }));
                        }
                      }}
                      className="p-1 rounded text-[var(--color-label-quaternary)] hover:text-red-400 hover:bg-red-500/10 transition-colors opacity-0 group-hover:opacity-100 shrink-0"
                       title={t("ui.delete-story", "Delete story")}
                       aria-label={t("ui.delete-world-name", "Delete {{name}}", { name: w.name })}
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>

                  {/* Sessions */}
                  {worldSessions.map((s) => {
                    const isActive =
                      s.id === currentSessionId &&
                      s.world_id === currentWorldId;
                    return (
                      <div key={s.id} className="group flex items-center gap-0.5">
                        <button
                          onClick={() => continueAdventure(s)}
                          className={`w-full text-left pl-7 pr-1 py-1.5 rounded-md text-[11px] transition-colors flex items-center gap-1.5 ${
                            isActive
                              ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)] font-medium"
                              : "text-[var(--color-label-secondary)] hover:bg-[var(--color-fill-quaternary)] hover:text-[var(--color-label-primary)]"
                          }`}
                        >
                          <MessageSquare className="w-3 h-3 shrink-0 opacity-60" />
                          <span className="truncate flex-1">
                            {s.summary || "Untitled"}
                          </span>
                          <span className="text-[9px] text-[var(--color-label-quaternary)] shrink-0">
                            {s.message_count ?? 0}
                          </span>
                        </button>
                        <button
                          onClick={() => {
                            if (window.confirm(t("ui.delete-session-confirm", "Delete this adventure? This cannot be undone."))) {
                              invoke("session_delete", { session_id: s.id })
                                .then(() => {
                                  if (currentSessionId === s.id) clearRuntimeState();
                                  return loadSessions();
                                })
                                .catch((error) => toast.error(t("ui.failed-to-delete-world", "Failed to delete adventure."), { description: String(error) }));
                            }
                          }}
                          className="p-1 rounded text-[var(--color-label-quaternary)] hover:text-red-400 hover:bg-red-500/10 transition-colors opacity-0 group-hover:opacity-100 shrink-0"
                          title={t("ui.delete-session", "Delete session")}
                          aria-label={t("ui.delete-session", "Delete session")}
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    );
                  })}

                  {/* New adventure quick action */}
                  <button
                    onClick={() => startNew(w.id)}
                    className="w-full text-left pl-7 pr-2 py-1 rounded-md text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] transition-colors flex items-center gap-1.5"
                  >
                    <Play className="w-3 h-3" />
                           {t("navigation.new_adventure", "New adventure")}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Bottom actions */}
      <div className="px-3 py-2 border-t border-[var(--color-separator)]/20 space-y-0.5">
        <button
          onClick={() => setActiveView("library")}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-[12px] text-[var(--color-label-secondary)] hover:bg-[var(--color-fill-quaternary)] hover:text-[var(--color-label-primary)] transition-colors"
        >
          <Library className="w-3.5 h-3.5" />
           {t("navigation.browse_stories", "Browse all stories")}
        </button>
        <button
          onClick={() => {
            useApp.getState().setEditingWorldId(null);
            setActiveView("world_new");
          }}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-[12px] text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] transition-colors font-medium"
        >
          <Plus className="w-3.5 h-3.5" />
           {t("navigation.new_story_short", "New Story")}
        </button>
      </div>

      {/* Footer */}
      <div className="px-3 py-2 border-t border-[var(--color-separator)]/20">
        {selectedModel && (
          <p className="text-[9px] text-[var(--color-label-quaternary)] truncate">
            {selectedModel}
          </p>
        )}
      </div>
    </div>
  );
}
