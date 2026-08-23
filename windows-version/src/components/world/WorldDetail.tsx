import { useEffect, useState } from "react";
import { ArrowLeft, BookOpen, Globe, Loader2, Edit3, Trash2, Download, Play, MessageSquare } from "lucide-react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";
import { genreGradient } from "@/lib/utils";
import type { CodexEntry, SessionSummary, Character, VisualBible } from "@/lib/tauri";

export function WorldDetail() {
  const { t } = useTranslation();
  const world = useApp((s) => s.selected_world_for_detail);
  const setActiveView = useApp((s) => s.setActiveView);
  const setActiveSession = useApp((s) => s.setActiveSession);
  const setEditingWorldId = useApp((s) => s.setEditingWorldId);
  const refreshWorlds = useApp((s) => s.refreshWorlds);
  const clearRuntimeState = useApp((s) => s.clearRuntimeState);

  const [codex, setCodex] = useState<CodexEntry[] | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [characters, setCharacters] = useState<Character[] | null>(null);
  const [codexError, setCodexError] = useState<string | null>(null);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  const [charactersError, setCharactersError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [characterForm, setCharacterForm] = useState(false);
  const [editChar, setEditChar] = useState<Character | null>(null);
  const [starting, setStarting] = useState(false);
  const [imgError, setImgError] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setImgError(false);
    setCodex(null);
    setSessions(null);
    setCharacters(null);
    setCodexError(null);
    setSessionsError(null);
    setCharactersError(null);
    if (!world) return () => { cancelled = true; };
    invoke<CodexEntry[]>("codex_list_for_world", { world_id: world.id })
      .then((entries) => { if (!cancelled) setCodex(entries); })
      .catch((error: unknown) => {
        if (!cancelled) setCodexError(errorMessage(error));
      });
    invoke<SessionSummary[]>("session_list_for_world", { world_id: world.id })
      .then((list) => { if (!cancelled) setSessions(list); })
      .catch((error: unknown) => {
        if (!cancelled) setSessionsError(errorMessage(error));
      });
    invoke<Character[]>("character_list", { world_id: world.id })
      .then((list) => { if (!cancelled) setCharacters(list); })
      .catch((error: unknown) => {
        if (!cancelled) setCharactersError(errorMessage(error));
      });
    return () => { cancelled = true; };
  }, [world?.id, loadAttempt]);

  // Reset character form when world changes
  useEffect(() => {
    setCharacterForm(false);
    setEditChar(null);
  }, [world?.id]);

  if (!world) {
    return (
      <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
        <div className="flex items-center justify-center py-20">
          <p className="text-[var(--color-label-tertiary)]">{t("ui.no-story-selected", "No story selected.")}</p>
        </div>
      </div>
    );
  }

  // Non-null shadow for TypeScript narrowing
  const w = world;

  async function startStory() {
    setStarting(true);
    try {
      const sessionId = await invoke<string>("session_create", { world_id: w.id });
      setActiveSession(sessionId, w.id);
      setActiveView("session");
    } catch (e) {
      setStarting(false);
      console.error("Failed to create session:", e);
      toast.error(t("ui.could-not-start-story", "Could not start this story."), { description: errorMessage(e) });
    }
  }

  function handleEdit() {
    setEditingWorldId(w.id);
    setActiveView("world_new");
  }

  async function handleExport() {
    const entries = codex || [];
    const visualBible = await invoke<VisualBible>("world_visual_bible_get", { world_id: w.id }).catch(() => ({ style: "", palette: "", character_anchors: "", location_anchors: "", negative_prompt: "" }));
    const data = JSON.stringify({
      schema_version: 1,
      world: { name: w.name, genre: w.genre, description: w.description, system_prompt: w.system_prompt, accent_color: w.accent_color, is_nsfw: w.is_nsfw },
      codex: entries.map(({ title, content, triggers }) => ({ title, content, triggers })),
      visual_bible: visualBible,
    }, null, 2);
    try {
      await navigator.clipboard.writeText(data);
      toast(t("ui.world-exported", "World exported"), { description: t("ui.world-data-copied", "\"{{name}}\" data copied to clipboard.", { name: w.name }), duration: 3000 });
    } catch (e) {
      console.error("export failed", e);
      toast.error(t("ui.world-export-failed", "World export failed"), { description: e instanceof Error ? e.message : String(e), duration: 4000 });
    }
  }

  async function handleDelete() {
    setDeleting(true);
    try {
      await invoke("world_delete", { id: w.id });
      clearRuntimeState();
      await refreshWorlds();
      useApp.getState().setActiveView("library");
    } catch (e) {
      console.error("Failed to delete world:", e);
      toast.error(t("ui.failed-to-delete-world", "Failed to delete world."), { description: String(e) });
      setDeleting(false);
      setConfirmDelete(false);
    }
  }

  const hasCover = !!w.cover_image_path && !imgError;
  const gradient = genreGradient(w.genre);

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
      {/* ── Header ──────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-6 py-3 border-b border-[var(--color-separator)]/40 shrink-0">
        <button
          onClick={() => setActiveView("library")}
          className="inline-flex items-center gap-2 text-[13px] font-medium text-[var(--color-label-secondary)] hover:text-[var(--color-label-primary)] transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          ← Stories
        </button>
        <div className="flex items-center gap-2">
          {confirmDelete ? (
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-[var(--color-system-red)] font-medium">{t("ui.delete", "Delete?")}</span>
              <button
                onClick={handleDelete}
                disabled={deleting}
                className="text-[11px] font-semibold text-[var(--color-system-red)] hover:underline disabled:opacity-50"
              >
                {deleting ? "..." : "Yes"}
              </button>
              <button
                onClick={() => setConfirmDelete(false)}
                className="text-[11px] text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)]"
              >
                {t("ui.no", "No")}
              </button>
            </div>
          ) : (
            <>
              <button
                onClick={handleExport}
                className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors"
                title={t("ui.export-world", "Export world")}
                aria-label={t("ui.export-world", "Export world")}
              >
                <Download className="w-3.5 h-3.5" />
              </button>
              <button
                onClick={() => setConfirmDelete(true)}
                className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-system-red)] hover:bg-red-500/10 transition-colors"
                title={t("ui.delete-world", "Delete world")}
                aria-label={t("ui.delete-world", "Delete world")}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
              <button
                onClick={handleEdit}
                className="inline-flex items-center gap-1.5 text-[12px] text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors"
              >
                <Edit3 className="w-3.5 h-3.5" />
                {t("ui.edit", "Edit")}
              </button>
            </>
          )}
        </div>
      </div>

      {/* ── Scrollable body ─────────────────────────────────────── */}
      <div className="flex-1 overflow-y-auto">
        {/* ── Hero cover ────────────────────────────────────────── */}
        <div className="w-full h-56 overflow-hidden bg-[var(--color-fill-quaternary)] relative">
          {hasCover ? (
            <img
              src={convertFileSrc(w.cover_image_path!)}
              alt={w.name}
              className="w-full h-full object-cover object-center"
              onError={() => setImgError(true)}
            />
          ) : (
            <div className={`w-full h-full bg-gradient-to-br ${gradient} flex flex-col items-center justify-center`}>
              <Globe className="w-12 h-12 text-[var(--color-label-quaternary)]" />
            </div>
          )}
          {/* Overlay gradient for text readability */}
          <div className="absolute inset-0 bg-gradient-to-t from-[var(--color-bg-content)] via-transparent to-transparent" />
          <div className="absolute bottom-0 left-0 right-0 p-6">
            <span className="text-[11px] font-medium tracking-[0.08em] uppercase text-[var(--color-accent)]">
              {w.genre || "Custom"}
            </span>
            <h1 className="text-[22px] font-display tracking-[0.02em] text-[var(--color-label-primary)] mt-1 leading-tight">
              {w.name}
            </h1>
          </div>

          {/* ── Play button (like AI Dungeon's New Game) ────────── */}
          <div className="absolute top-4 right-4 flex items-center gap-2">
            {/* Continue latest session if one exists */}
            {sessions && sessions.some((session) => !session.is_checkpoint) && (
              <button
                onClick={async () => {
                   const latest = sessions.find((session) => !session.is_checkpoint)!;
                  setActiveSession(latest.id, latest.world_id);
                  setActiveView("session");
                }}
                className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-[var(--color-fill-quaternary)]/80 backdrop-blur-lg border border-white/10 text-[var(--color-label-primary)] text-[12px] font-semibold hover:bg-[var(--color-fill-tertiary)] transition-colors"
              >
                <MessageSquare className="w-3.5 h-3.5" />
                {t("ui.continue", "Continue")}
              </button>
            )}
            <button
              onClick={startStory}
              disabled={starting}
              className="inline-flex items-center gap-2 px-5 py-2 rounded-lg bg-[var(--color-accent)] text-black text-[12px] font-bold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors disabled:opacity-50 shadow-lg"
            >
              {starting ? (
                <><Loader2 className="w-3.5 h-3.5 animate-spin" /> {t("ui.starting", "Starting…")}</>
              ) : (
                <><Play className="w-3.5 h-3.5" fill="currentColor" /> {t("ui.play", "Play")}</>
              )}
            </button>
          </div>
        </div>

        <div className="px-6 py-5 space-y-6 max-w-3xl mx-auto">
          {/* ── Description ──────────────────────────────────────── */}
          {w.description && (
            <div>
              <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-2">{t("ui.description", "Description")}</h3>
              <p className="text-[14px] font-serif leading-[1.75] text-[var(--color-label-primary)] whitespace-pre-wrap">
                {w.description}
              </p>
            </div>
          )}

          {/* ── System prompt ────────────────────────────────────── */}
          <div className="rounded-xl bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)] p-4">
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-2">{t("ui.system-prompt", "System Prompt")}</h3>
            <p className="text-[13px] font-mono leading-relaxed text-[var(--color-label-secondary)] whitespace-pre-wrap">
              {w.system_prompt}
            </p>
          </div>

          {/* ── Codex entries ────────────────────────────────────── */}
          <div>
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-3">
              Codex Entries
              {codex && <span className="ml-2 text-[var(--color-label-tertiary)]">· {codex.length}</span>}
            </h3>

            {codex === null && !codexError && (
              <div className="flex items-center justify-center py-8">
                <Loader2 className="w-4 h-4 animate-spin text-[var(--color-label-tertiary)]" />
              </div>
            )}

            {codexError && <DataError message={codexError} onRetry={() => setLoadAttempt((attempt) => attempt + 1)} />}

            {codex && !codexError && codex.length === 0 && (
              <div className="text-center py-8 rounded-xl border border-dashed border-[var(--color-separator)]/30">
                <p className="text-[12px] text-[var(--color-label-tertiary)] mb-3">
                  {t("ui.no-codex-entries-for-this-world", "No codex entries for this world.")}
                </p>
                <button
                  onClick={() => {
                    setEditingWorldId(w.id);
                    setActiveView("world_new");
                  }}
                  className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-[var(--color-accent)] text-black text-[12px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors"
                >
                  {t("ui.add-entry", "Add entry")}
                </button>
              </div>
            )}

            {codex && !codexError && codex.length > 0 && (
              <div className="space-y-3">
                {codex.map((entry) => (
                  <details
                    key={entry.id}
                    className="group rounded-xl border border-[var(--color-separator)]/30 bg-[var(--color-fill-quaternary)]/40 overflow-hidden"
                  >
                    <summary className="flex items-center justify-between px-4 py-3 cursor-pointer text-[13px] font-medium text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors list-none">
                      <span className="font-display tracking-[0.02em]">{entry.title}</span>
                      <span className="text-[10px] text-[var(--color-label-tertiary)] font-mono tracking-wider">
                        {entry.triggers && entry.triggers !== "[]" ? entry.triggers.slice(0, 40) + (entry.triggers.length > 40 ? "…" : "") : ""}
                      </span>
                    </summary>
                    <div className="px-4 pb-4 pt-1">
                      <p className="text-[13px] font-serif leading-relaxed text-[var(--color-label-secondary)] whitespace-pre-wrap">
                        {entry.content}
                      </p>
                    </div>
                  </details>
                ))}
              </div>
            )}
          </div>

          {/* ── Characters ────────────────────────────────────────── */}
          <div>
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-3">
              Characters
              {characters && <span className="ml-2 text-[var(--color-label-tertiary)]">· {characters.length}</span>}
            </h3>

            {characters === null && !charactersError && (
              <div className="flex items-center justify-center py-4">
                <Loader2 className="w-4 h-4 animate-spin text-[var(--color-label-tertiary)]" />
              </div>
            )}

            {charactersError && <DataError message={charactersError} onRetry={() => setLoadAttempt((attempt) => attempt + 1)} />}

            {characters && !charactersError && characters.length === 0 && !characterForm && (
              <div className="text-center py-6 rounded-xl border border-dashed border-[var(--color-separator)]/30">
                <p className="text-[12px] text-[var(--color-label-tertiary)] mb-3">
                  {t("ui.no-characters-yet-add-characters-to-define-who-l", "No characters yet. Add characters to define who lives in this world.")}
                </p>
                <button
                  onClick={() => setCharacterForm(true)}
                  className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-[var(--color-accent)] text-black text-[12px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors"
                >
                  {t("ui.add-character", "Add character")}
                </button>
              </div>
            )}

            {characterForm && (
              <CharacterFormInline
                worldId={w.id}
                editChar={editChar}
                onSaved={() => {
                  setCharacterForm(false);
                  setEditChar(null);
                  invoke<Character[]>("character_list", { world_id: w.id }).then(setCharacters).catch(() => {});
                }}
                onCancel={() => {
                  setCharacterForm(false);
                  setEditChar(null);
                }}
              />
            )}

            {characters && !charactersError && characters.length > 0 && (
              <div className="space-y-2">
                {characters.map((c) => (
                  <CharacterCard
                    key={c.id}
                    character={c}
                    onEdit={() => {
                      setEditChar(c);
                      setCharacterForm(true);
                    }}
                    onDelete={async () => {
                      await invoke("character_delete", { id: c.id });
                      invoke<Character[]>("character_list", { world_id: w.id }).then(setCharacters).catch(() => {});
                    }}
                  />
                ))}
              </div>
            )}
          </div>

          {/* ── Sessions ────────────────────────────────────────── */}
          <div>
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-3">
              Sessions
              {sessions && <span className="ml-2 text-[var(--color-label-tertiary)]">· {sessions.length}</span>}
            </h3>

            {sessions === null && !sessionsError && (
              <div className="flex items-center justify-center py-4">
                <Loader2 className="w-4 h-4 animate-spin text-[var(--color-label-tertiary)]" />
              </div>
            )}

            {sessionsError && <DataError message={sessionsError} onRetry={() => setLoadAttempt((attempt) => attempt + 1)} />}

            {sessions && !sessionsError && sessions.length === 0 && (
              <div className="text-center py-6 rounded-xl border border-dashed border-[var(--color-separator)]/30">
                <p className="text-[12px] text-[var(--color-label-tertiary)] mb-3">
                  {t("ui.no-sessions-yet-start-a-story-to-begin", "No sessions yet. Start a story to begin.")}
                </p>
                <button
                  onClick={async () => {
                    try {
                      const sessionId = await invoke<string>("session_create", { world_id: w.id });
                      useApp.getState().setActiveSession(sessionId, w.id);
                      setActiveView("session");
                    } catch (error) {
                      toast.error(t("ui.could-not-start-story", "Could not start this story."), { description: errorMessage(error) });
                    }
                  }}
                  className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-[var(--color-accent)] text-black text-[12px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors"
                >
                  {t("ui.start-story", "Start story")}
                </button>
              </div>
            )}

            {sessions && sessions.length > 0 && (
              <div className="space-y-2">
                {sessions.map((s) => (
                  <SessionRow key={s.id} session={s} worldId={w.id} />
                ))}
              </div>
            )}
          </div>

          {/* ── Actions ──────────────────────────────────────────── */}
          <div className="flex items-center gap-3 pt-2 pb-6">
            <button
              onClick={startStory}
              disabled={starting}
              className="inline-flex items-center gap-2 px-6 py-3 rounded-xl bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] text-black font-semibold text-[13px] transition-all disabled:opacity-50"
            >
              {starting ? (
                <><Loader2 className="w-4 h-4 animate-spin" /> {t("ui.starting", "Starting…")}</>
              ) : (
                <><BookOpen className="w-4 h-4" /> {t("ui.start-story", "Start story")}</>
              )}
            </button>
            <button
              onClick={handleEdit}
              className="inline-flex items-center gap-2 px-5 py-3 rounded-xl border border-[var(--color-separator)] text-[var(--color-label-secondary)] hover:text-[var(--color-label-primary)] hover:border-[var(--color-label-tertiary)] text-[13px] font-medium transition-all"
            >
              <Edit3 className="w-4 h-4" />
              {t("ui.edit-world", "Edit world")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function DataError({ message, onRetry }: { message: string; onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center gap-2 rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-5 text-center">
      <p className="text-[12px] text-[var(--color-system-red)]">{t("ui.could-not-load-this-section", "Could not load this section.")}</p>
      <p className="max-w-md break-words text-[10px] text-[var(--color-label-tertiary)]">{message}</p>
      <button
        type="button"
        onClick={onRetry}
        className="text-[11px] font-semibold text-[var(--color-accent)] underline underline-offset-2 hover:no-underline"
      >
        {t("ui.try-again", "Try again")}
      </button>
    </div>
  );
}

function SessionRow({ session, worldId }: { session: SessionSummary; worldId: string }) {
  const { t } = useTranslation();
  const setActiveSession = useApp((s) => s.setActiveSession);
  const setActiveView = useApp((s) => s.setActiveView);
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState(session.summary || "");
  const [continuing, setContinuing] = useState(false);
  const displayName = session.summary || new Date(session.created_at).toLocaleDateString();

  async function handleRename() {
    const trimmed = nameDraft.trim();
    if (trimmed && trimmed !== session.summary) {
      try {
        await invoke("session_update_summary", { session_id: session.id, summary: trimmed });
      } catch {
        toast.error(t("ui.rename-failed", "Failed to rename session"));
      }
    }
    setRenaming(false);
  }

  return (
    <div className="flex items-center justify-between px-4 py-3 rounded-xl bg-[var(--color-fill-quaternary)]/40 border border-[var(--color-separator)]/30">
      <div className="flex items-center gap-3 min-w-0 flex-1">
        {renaming ? (
          <input
            type="text"
            value={nameDraft}
            onChange={(e) => setNameDraft(e.target.value)}
            onBlur={handleRename}
            onKeyDown={(e) => { if (e.key === "Enter") handleRename(); if (e.key === "Escape") setRenaming(false); }}
            className="w-40 text-[12px] px-2 py-1 rounded bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] outline-none"
            autoFocus
          />
        ) : (
          <button
            onClick={() => { setNameDraft(session.summary || ""); setRenaming(true); }}
            className="text-[12px] text-[var(--color-label-primary)] font-medium hover:text-[var(--color-accent)] transition-colors truncate text-left"
            title={t("ui.rename", "Rename")}
            aria-label={t("ui.rename-story", "Rename story")}
          >
            {displayName}
          </button>
        )}
        <span className="text-[10px] text-[var(--color-label-tertiary)] shrink-0">
          {session.message_count} msg{session.message_count !== 1 ? "s" : ""}
        </span>
      </div>
      <button
        onClick={async () => {
          setContinuing(true);
          setActiveSession(session.id, worldId);
          setActiveView("session");
        }}
        disabled={continuing}
        className="text-[11px] font-medium text-[var(--color-accent)] hover:text-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors disabled:opacity-50 shrink-0"
      >
        {continuing ? "..." : "Continue"}
      </button>
    </div>
  );
}

// ─── Character Card ─────────────────────────────────────────────
function CharacterCard({ character, onEdit, onDelete }: { character: Character; onEdit: () => void; onDelete: () => Promise<void> }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [deleting, setDeleting] = useState(false);

  let traitsList: string[] = [];
  try { traitsList = JSON.parse(character.traits); } catch { /* ignore */ }

  const roleBadgeColor: Record<string, string> = {
    player: "bg-blue-500/20 text-blue-400",
    npc: "bg-purple-500/20 text-purple-400",
    narrator: "bg-amber-500/20 text-amber-400",
  };
  const badgeClass = roleBadgeColor[character.role] ?? "bg-gray-500/20 text-gray-400";

  const initials = character.name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);

  return (
    <div className="rounded-xl border border-[var(--color-separator)]/30 bg-[var(--color-fill-quaternary)]/40 overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3">
        <div className="flex items-center gap-3 min-w-0 flex-1">
          {/* Avatar placeholder */}
          <div className="w-8 h-8 rounded-full bg-[var(--color-accent)]/20 flex items-center justify-center shrink-0">
            <span className="text-[11px] font-semibold text-[var(--color-accent)]">{initials}</span>
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-[13px] font-medium text-[var(--color-label-primary)] truncate">{character.name}</span>
              <span className={`text-[9px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded ${badgeClass}`}>{character.role}</span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <button onClick={() => setExpanded(!expanded)} className="p-1 rounded text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors text-[11px]">
            {expanded ? "Collapse" : "Details"}
          </button>
          <button onClick={onEdit} className="p-1 rounded text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)] transition-colors" title={t("ui.edit", "Edit")} aria-label={t("ui.edit-character", "Edit character")}>
            <Edit3 className="w-3 h-3" />
          </button>
          <button
            onClick={async () => {
              setDeleting(true);
              try {
                await onDelete();
              } catch {
                toast.error(t("ui.delete-character-failed", "Failed to delete character"));
              } finally {
                setDeleting(false);
              }
            }}
            disabled={deleting}
            className="p-1 rounded text-[var(--color-label-tertiary)] hover:text-[var(--color-system-red)] transition-colors"
            title={t("ui.delete-2", "Delete")}
            aria-label={t("ui.delete-character", "Delete character")}
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      </div>
      {expanded && (
        <div className="px-4 pb-4 pt-1 space-y-3 border-t border-[var(--color-separator)]/20">
          {character.backstory && (
            <div>
              <h4 className="text-[10px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-1">{t("ui.backstory", "Backstory")}</h4>
              <p className="text-[12px] font-serif leading-relaxed text-[var(--color-label-secondary)] whitespace-pre-wrap">{character.backstory}</p>
            </div>
          )}
          {traitsList.length > 0 && (
            <div>
              <h4 className="text-[10px] font-semibold uppercase tracking-[0.1em] text-[var(--color-label-tertiary)] mb-1">{t("ui.traits", "Traits")}</h4>
              <div className="flex flex-wrap gap-1.5">
                {traitsList.map((trait, i) => (
                  <span key={i} className="text-[10px] px-2 py-0.5 rounded-full bg-[var(--color-fill-quaternary)] border border-[var(--color-separator)]/20 text-[var(--color-label-secondary)]">
                    {trait}
                  </span>
                ))}
              </div>
            </div>
          )}
          {!character.backstory && traitsList.length === 0 && (
            <p className="text-[11px] text-[var(--color-label-tertiary)] italic">{t("ui.no-details-set", "No details set.")}</p>
          )}
        </div>
      )}
    </div>
  );
}

// ─── Character Inline Form ──────────────────────────────────────
function CharacterFormInline({ worldId, editChar, onSaved, onCancel }: { worldId: string; editChar: Character | null; onSaved: () => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const [name, setName] = useState(editChar?.name ?? "");
  const [role, setRole] = useState(editChar?.role ?? "npc");
  const [backstory, setBackstory] = useState(editChar?.backstory ?? "");
  const [traitsText, setTraitsText] = useState(() => {
    if (editChar) {
      try { return (JSON.parse(editChar.traits) as string[]).join(", "); } catch { return ""; }
    }
    return "";
  });
  const [saving, setSaving] = useState(false);

  async function handleSave() {
    if (!name.trim()) return;
    setSaving(true);
    try {
      const traits = traitsText
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);
      if (editChar) {
        await invoke("character_update", {
          id: editChar.id,
          name: name.trim() || null,
          role: role || null,
          backstory: backstory.trim() || null,
          traits: traits.length > 0 ? traits : null,
          avatar_path: null,
        });
      } else {
        await invoke("character_create", {
          world_id: worldId,
          name: name.trim(),
          role: role || null,
          backstory: backstory.trim() || null,
          traits: traits.length > 0 ? traits : null,
          avatar_path: null,
        });
      }
      onSaved();
    } catch (e) {
      console.error("Failed to save character:", e);
      toast.error(t("ui.failed-to-save-character", "Failed to save character."), { description: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="rounded-xl border border-[var(--color-separator)]/30 bg-[var(--color-fill-quaternary)]/60 p-4 space-y-3">
      <h4 className="text-[11px] font-semibold text-[var(--color-label-primary)]">{editChar ? "Edit character" : "New character"}</h4>

      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2 sm:col-span-1">
          <label className="text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--color-label-tertiary)] block mb-1">{t("ui.name", "Name")}</label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("ui.character-name", "Character name")}
            className="w-full text-[12px] px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)]/30 text-[var(--color-label-primary)] outline-none focus:border-[var(--color-accent)] transition-colors"
          />
        </div>
        <div className="col-span-2 sm:col-span-1">
          <label className="text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--color-label-tertiary)] block mb-1">{t("ui.role", "Role")}</label>
          <select
            value={role}
            onChange={(e) => setRole(e.target.value)}
            className="w-full text-[12px] px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)]/30 text-[var(--color-label-primary)] outline-none focus:border-[var(--color-accent)] transition-colors"
          >
            <option value="player">{t("ui.player", "Player")}</option>
            <option value="npc">{t("ui.npc", "NPC")}</option>
            <option value="narrator">{t("ui.narrator", "Narrator")}</option>
          </select>
        </div>
      </div>

      <div>
        <label className="text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--color-label-tertiary)] block mb-1">{t("ui.backstory", "Backstory")}</label>
        <textarea
          value={backstory}
          onChange={(e) => setBackstory(e.target.value)}
          placeholder={t("ui.backstory-and-background", "Backstory and background...")}
          rows={3}
          className="w-full text-[12px] px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)]/30 text-[var(--color-label-primary)] outline-none focus:border-[var(--color-accent)] transition-colors resize-none"
        />
      </div>

      <div>
        <label className="text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--color-label-tertiary)] block mb-1">{t("ui.traits-comma-separated", "Traits (comma-separated)")}</label>
        <input
          type="text"
          value={traitsText}
          onChange={(e) => setTraitsText(e.target.value)}
          placeholder="brave, curious, impulsive"
          className="w-full text-[12px] px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)]/30 text-[var(--color-label-primary)] outline-none focus:border-[var(--color-accent)] transition-colors"
        />
      </div>

      <div className="flex items-center justify-end gap-2 pt-1">
        <button
          onClick={onCancel}
          className="text-[11px] font-medium text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] transition-colors px-3 py-1.5"
        >
          {t("ui.cancel", "Cancel")}
        </button>
        <button
          onClick={handleSave}
          disabled={saving || !name.trim()}
          className="inline-flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-[var(--color-accent)] text-black text-[11px] font-semibold hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] transition-colors disabled:opacity-50"
        >
          {saving ? "Saving..." : editChar ? "Update" : "Create"}
        </button>
      </div>
    </div>
  );
}
