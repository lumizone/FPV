import { useEffect, useRef, useState } from "react";
import {
  AlertCircle,
  ArrowLeft,
  BookOpen,
  Check,
  Globe,
  Loader2,
  Plus,
  Trash2,
  Upload,
  Wand2,
} from "lucide-react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { VisualBible } from "@/lib/tauri";

// ── Types ──────────────────────────────────────────────────────────

interface CodexEntryForm {
  /** Client-side key for the entry while it hasn't been persisted yet
   *  (empty string = server-persisted entry). */
  key: string;
  id?: string;
  title: string;
  content: string;
  triggers: string; // comma-separated, stored as JSON array
}

interface FormData {
  name: string;
  genre: string;
  description: string;
  system_prompt: string;
  is_nsfw: boolean;
  accent_color: string;
  privacy_mode: "local_only" | "cloud_allowed";
}

type SubmitPhase =
  | "idle"
  | "creating_world"
  | "creating_codex"
  | "success"
  | "error";

// ── Helpers ────────────────────────────────────────────────────────

let _keyCounter = 0;
function nextKey(): string { return `new_${++_keyCounter}`; }

/** Convert comma-separated triggers string to a JSON array string. */
function triggersToJson(raw: string): string {
  const parts = raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return JSON.stringify(parts);
}

// ── Component ──────────────────────────────────────────────────────

/**
 * A full-screen form for creating a new story world.
 *
 * Collects the world's metadata (name, genre, description, system prompt,
 * NSFW flag) plus an optional set of codex entries. On submit it:
 *  1. Calls `world_save` to persist the world and all editor data atomically.
 *  2. Shows a "Generate cover" button after success.
 */
export function WorldCreator() {
  const { t } = useTranslation();
  const setActiveView = useApp((s) => s.setActiveView);
  const editingWorldId = useApp((s) => s.editing_world_id);
  const setEditingWorldId = useApp((s) => s.setEditingWorldId);
  const refreshWorlds = useApp((s) => s.refreshWorlds);
  const isEditing = editingWorldId !== null;

  // ── Form state ───────────────────────────────────────────────────
  const [form, setForm] = useState<FormData>({
    name: "",
    genre: "",
    description: "",
    system_prompt: "",
    is_nsfw: false,
    accent_color: "",
    privacy_mode: "local_only",
  });

  const [codexEntries, setCodexEntries] = useState<CodexEntryForm[]>([]);
  const [removedCodexIds, setRemovedCodexIds] = useState<string[]>([]);
  const [existingCoverPath, setExistingCoverPath] = useState<string | null>(null);
  const [visualBible, setVisualBible] = useState<VisualBible>({ style: "", palette: "", character_anchors: "", location_anchors: "", negative_prompt: "" });
  const [loadingWorld, setLoadingWorld] = useState(isEditing);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);

  // ── Load existing world for editing ─────────────────────────────
  useEffect(() => {
    if (!editingWorldId) return;
    let cancelled = false;
    setLoadingWorld(true);
    setLoadError(null);
    Promise.all([
       invoke<{ name: string; genre: string; description: string | null; system_prompt: string; is_nsfw: boolean; accent_color: string | null; cover_image_path: string | null; privacy_mode: "local_only" | "cloud_allowed" }>("world_get", { id: editingWorldId }),
      invoke<{ id: string; title: string; content: string; triggers: string }[]>("codex_list_for_world", { world_id: editingWorldId }),
      invoke<VisualBible>("world_visual_bible_get", { world_id: editingWorldId }),
    ])
      .then(([world, entries, bible]) => {
        if (cancelled) return;
        setForm({
          name: world.name,
          genre: world.genre,
          description: world.description || "",
          system_prompt: world.system_prompt,
          is_nsfw: world.is_nsfw,
          accent_color: world.accent_color || "",
          privacy_mode: world.privacy_mode || "local_only",
        });
        setExistingCoverPath(world.cover_image_path);
        setVisualBible(bible);
        setRemovedCodexIds([]);
         setCodexEntries(
            entries.map((e) => ({
              key: e.id,
              id: e.id,
              title: e.title,
              content: e.content,
              triggers: (() => {
                try { return JSON.parse(e.triggers).join(", "); } catch { return e.triggers; }
              })(),
            }))
           );
        setLoadingWorld(false);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setLoadingWorld(false);
        setLoadError(error instanceof Error ? error.message : String(error));
      });
    return () => { cancelled = true; };
  }, [editingWorldId, loadAttempt]);

  // ── Submission state ─────────────────────────────────────────────
  const [phase, setPhase] = useState<SubmitPhase>("idle");
  const [savedWasEdit, setSavedWasEdit] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [newWorldId, setNewWorldId] = useState<string | null>(null);
  const [generatingCover, setGeneratingCover] = useState(false);
  const [coverPath, setCoverPath] = useState<string | null>(null);

  // ── Validation ───────────────────────────────────────────────────
  const nameMissing = form.name.trim().length === 0;
  const promptMissing = form.system_prompt.trim().length === 0;
  const canSubmit =
    !nameMissing && !promptMissing && phase === "idle";

  // ── Derived booleans ────────────────────────────────────────────
  const isSubmitting = phase === "creating_world" || phase === "creating_codex";
  function updateField<K extends keyof FormData>(key: K, value: FormData[K]) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  function addCodexEntry() {
    setCodexEntries((prev) => [
      ...prev,
      { key: nextKey(), title: "", content: "", triggers: "" },
    ]);
  }

  function updateCodexEntry(key: string, patch: Partial<CodexEntryForm>) {
    setCodexEntries((prev) =>
      prev.map((e) => (e.key === key ? { ...e, ...patch } : e)),
    );
  }

  function removeCodexEntry(key: string) {
    const entry = codexEntries.find((item) => item.key === key);
    if (entry?.id) setRemovedCodexIds((prev) => [...prev, entry.id!]);
    setCodexEntries((prev) => prev.filter((e) => e.key !== key));
  }

  function cancelEditing() {
    setEditingWorldId(null);
    setActiveView("library");
  }

  // ── Submit ───────────────────────────────────────────────────────
  async function handleSubmit() {
    if (!canSubmit) return;

    setPhase("creating_world");
    setErrorMsg(null);

    try {
      // 1. Create or update the world
      const payload = {
        name: form.name.trim(),
        genre: form.genre.trim() || "custom",
        description: form.description.trim() || null,
        system_prompt: form.system_prompt.trim(),
        accent_color: form.accent_color.trim() || null,
        is_nsfw: form.is_nsfw,
        cover_image_path: isEditing ? existingCoverPath : null,
        source: "user",
      };

      const worldId = await invoke<string>("world_save", {
        id: isEditing ? editingWorldId : newWorldId,
        w: payload,
        visual_bible: visualBible,
        codex_entries: codexEntries
          .filter((entry) => entry.title.trim())
          .map((entry) => ({
            id: entry.id || null,
            title: entry.title.trim(),
            content: entry.content.trim(),
            triggers: triggersToJson(entry.triggers),
          })),
        removed_codex_ids: removedCodexIds,
        privacy_mode: form.privacy_mode,
      });

      setNewWorldId(worldId);
      setCoverPath(isEditing ? existingCoverPath : null);

      setSavedWasEdit(isEditing);
      setPhase("success");
      setEditingWorldId(null);
      await refreshWorlds();
    } catch (e: unknown) {
      setPhase("error");
      setErrorMsg(
        (e as { message?: string })?.message ?? String(e),
      );
    }
  }

  // ── Generate cover ───────────────────────────────────────────────
  async function handleGenerateCover() {
    if (!newWorldId) return;
    setGeneratingCover(true);
    try {
      const path = await invoke<string>("world_generate_cover", {
        world_id: newWorldId,
      });
      setCoverPath(path);
    } catch (e: unknown) {
      console.error("Cover generation failed:", e);
      toast.error(t("ui.cover-generation-failed", "Cover generation failed. Install an image model in Settings → Image Model first."));
    } finally {
      setGeneratingCover(false);
    }
  }

  // ── Upload cover ─────────────────────────────────────────────────
  const fileInputRef = useRef<HTMLInputElement>(null);
  async function handleUploadCover(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file || !newWorldId) return;
    const maxCoverBytes = 10 * 1024 * 1024;
    if (!file.type.startsWith("image/") || file.size > maxCoverBytes) {
      toast.error(t("ui.cover-upload-failed", "Cover upload failed."), { description: "Use a PNG, JPEG, or WebP image up to 10 MB." });
      e.target.value = "";
      return;
    }
    setGeneratingCover(true);
    try {
      const b64 = await fileToBase64(file);
      const path = await invoke<string>("world_upload_cover", {
        world_id: newWorldId,
        image_b64: b64,
      });
      setCoverPath(path);
    } catch (err: unknown) {
      console.error("Cover upload failed:", err);
      toast.error(t("ui.cover-upload-failed", "Cover upload failed."));
    } finally {
      setGeneratingCover(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  function fileToBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        // Strip the data-URI prefix; the Rust side expects raw base64.
        const comma = result.indexOf(",");
        resolve(comma >= 0 ? result.slice(comma + 1) : result);
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }

  // ── Success screen ───────────────────────────────────────────────
  if (phase === "success") {
    return (
      <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-separator)]/40 shrink-0">
          <h2 className="text-[16px] font-display tracking-[0.04em] flex items-center gap-2">
            <BookOpen className="w-4 h-4 text-[var(--color-system-green)]" />
            {savedWasEdit ? t("ui.story-updated", "Story Updated") : t("ui.story-created", "Story Created")}
          </h2>
        </div>
        <div className="flex-1 flex flex-col items-center justify-center px-6 text-center">
          <div className="w-16 h-16 rounded-full bg-[var(--color-system-green)]/20 flex items-center justify-center mb-4">
            <Check className="w-8 h-8 text-[var(--color-system-green)]" />
          </div>
          <h3 className="text-[18px] font-semibold text-[var(--color-label-primary)] mb-2">
            "{form.name}" is ready
          </h3>
          <p className="text-[13px] text-[var(--color-label-secondary)] max-w-md mb-8">
            {t("ui.your-story-has-been-saved-start-playing-or-add-a", "Your story has been saved. Start playing or add a cover image below.")}
          </p>

          {/* Cover image preview / generate / upload */}
          <div className="mb-8 flex flex-col items-center gap-3">
            {coverPath && (
              <div className="rounded-xl overflow-hidden border border-[var(--color-separator)]/30 w-64 aspect-[16/10]">
                <img
                  src={convertFileSrc(coverPath)}
                  alt="Story cover"
                  className="w-full h-full object-cover"
                />
              </div>
            )}
            <div className="flex items-center gap-3">
              <button
                onClick={handleGenerateCover}
                disabled={generatingCover}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-xl border-2 border-dashed border-[var(--color-separator)] hover:border-[var(--color-accent)]/50 text-[var(--color-label-secondary)] hover:text-[var(--color-accent)] transition-colors text-[13px] font-medium disabled:opacity-50"
              >
                {generatingCover ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" />
                    {t("ui.working", "Working...")}
                  </>
                ) : (
                  <>
                    <Wand2 className="w-4 h-4" />
                    {t("ui.generate-cover", "Generate cover")}
                  </>
                )}
              </button>
              <input
                ref={fileInputRef}
                type="file"
                accept="image/png,image/jpeg,image/webp"
                onChange={handleUploadCover}
                className="hidden"
              />
              <button
                onClick={() => fileInputRef.current?.click()}
                disabled={generatingCover}
                className="inline-flex items-center gap-2 px-5 py-2.5 rounded-xl border-2 border-dashed border-[var(--color-separator)] hover:border-[var(--color-accent)]/50 text-[var(--color-label-secondary)] hover:text-[var(--color-accent)] transition-colors text-[13px] font-medium disabled:opacity-50"
              >
                <Upload className="w-4 h-4" />
                {t("ui.upload-cover", "Upload cover")}
              </button>
            </div>
          </div>

          {/* Navigation */}
          <div className="flex gap-3">
            <button
              onClick={() => setActiveView("library")}
              className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg bg-[var(--color-fill-tertiary)] hover:bg-[var(--color-fill-secondary)] text-[var(--color-label-primary)] text-[13px] font-medium transition-colors"
            >
              <ArrowLeft className="w-4 h-4" />
              {t("ui.back-to-library", "Back to library")}
            </button>
            <button
              onClick={async () => {
                if (!newWorldId) return setActiveView("library");
                try {
                  const sessionId = await invoke<string>("session_create", { world_id: newWorldId });
                  useApp.getState().setActiveSession(sessionId, newWorldId);
                  setActiveView("session");
                } catch {
                  setActiveView("library");
                }
              }}
              className="inline-flex items-center gap-2 px-5 py-2.5 rounded-lg bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] text-black text-[13px] font-semibold transition-colors"
            >
              {t("ui.start-story", "Start story")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  // ── Loading during edit fetch ────────────────────────────────────
  if (loadingWorld) {
    return (
      <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
        <div className="flex items-center justify-center py-20">
          <Loader2 className="w-5 h-5 animate-spin text-[var(--color-label-tertiary)]" />
        </div>
      </div>
    );
  }

  if (loadError) {
    return (
      <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
        <div className="flex flex-col items-center justify-center gap-4 px-6 py-20 text-center">
          <AlertCircle className="w-6 h-6 text-[var(--color-system-red)]" />
          <p className="max-w-md text-[13px] text-[var(--color-label-secondary)]">
            {t("ui.could-not-load-this-story-for-editing", "Could not load this story for editing.")}
          </p>
          <p className="max-w-md text-[11px] text-[var(--color-label-tertiary)] break-words">
            {loadError}
          </p>
          <button
            type="button"
            onClick={() => setLoadAttempt((attempt) => attempt + 1)}
            className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-[12px] font-semibold text-black transition-colors hover:brightness-110"
          >
            {t("ui.try-again", "Try again")}
          </button>
        </div>
      </div>
    );
  }

  // ── Main form ────────────────────────────────────────────────────
  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-[var(--color-bg-content)]">
      {/* ── Header ─────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-separator)]/40 shrink-0">
        <div className="flex items-center gap-3">
          <button
            onClick={cancelEditing}
            className="p-1.5 rounded-lg text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
            aria-label={t("ui.back-to-library", "Back to library")}
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <h2 className="text-[16px] font-display tracking-[0.04em] flex items-center gap-2">
            <Globe className="w-4 h-4 text-[var(--color-accent)]" />
            {isEditing ? "Edit Story" : "Create New Story"}
          </h2>
        </div>
      </div>

      {/* ── Scrollable form body ───────────────────────────────── */}
      <div className="flex-1 overflow-y-auto p-6">
        {/* Error banner */}
        {phase === "error" && errorMsg && (
          <div className="bg-red-500/10 border border-red-500/20 rounded-xl p-3 text-[12px] text-[var(--color-system-red)] mb-6">
            <div className="flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>Failed to create world: {errorMsg}</span>
            </div>
            <button
              type="button"
              onClick={() => {
                setPhase("idle");
                setErrorMsg(null);
              }}
              className="mt-2 text-[11px] font-semibold underline underline-offset-2 hover:no-underline"
            >
              {t("ui.try-again", "Try again")}
            </button>
          </div>
        )}

        <div className="max-w-2xl mx-auto space-y-6">
          {/* ── Name ────────────────────────────────────────── */}
          <div>
            <label className="block text-[13px] font-medium text-[var(--color-label-secondary)] mb-1.5">
              {t("ui.name", "Name")} <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              value={form.name}
              onChange={(e) => updateField("name", e.target.value)}
              placeholder="e.g. Eldoria, Neo Tokyo, Dark Hollow"
              className={`w-full px-3 py-2 rounded-lg bg-[var(--color-fill-secondary)] border ${
                nameMissing && phase !== "idle"
                  ? "border-red-500/50"
                  : "border-[var(--color-separator)]"
              } text-[var(--color-label-primary)] text-[14px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors`}
            />
          </div>

          {/* ── Genre ────────────────────────────────────────── */}
          <div>
            <label className="block text-[13px] font-medium text-[var(--color-label-secondary)] mb-1.5">
              {t("ui.genre", "Genre")}
            </label>
            <div className="flex flex-wrap gap-1.5 mb-2">
              {(["fantasy", "scifi", "romance", "horror", "mystery", "adventure", "drama", "comedy"] as const).map((g) => (
                <button
                  key={g}
                  type="button"
                  onClick={() => updateField("genre", form.genre === g ? "" : g)}
                  className={`px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all border ${
                    form.genre === g
                      ? "bg-[var(--color-accent)] text-black border-[var(--color-accent)]"
                      : "bg-[var(--color-fill-quaternary)] text-[var(--color-label-secondary)] border-[var(--color-separator)] hover:border-[var(--color-label-tertiary)]"
                  }`}
                >
                  {g === "scifi" ? "Sci-Fi" : g.charAt(0).toUpperCase() + g.slice(1)}
                </button>
              ))}
              <button
                type="button"
                onClick={() => updateField("genre", "custom")}
                className={`px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all border ${
                  !["fantasy", "scifi", "romance", "horror", "mystery", "adventure", "drama", "comedy"].includes(form.genre) && form.genre
                    ? "bg-[var(--color-accent)] text-black border-[var(--color-accent)]"
                    : "bg-[var(--color-fill-quaternary)] text-[var(--color-label-secondary)] border-[var(--color-separator)] hover:border-[var(--color-label-tertiary)]"
                }`}
              >
                {t("ui.custom", "Custom…")}
              </button>
            </div>
            {!["fantasy", "scifi", "romance", "horror", "mystery", "adventure", "drama", "comedy"].includes(form.genre) && (
              <input
                type="text"
                value={form.genre === "custom" ? "" : form.genre}
                onChange={(e) => updateField("genre", e.target.value || "custom")}
                placeholder={t("ui.type-your-genre", "Type your genre...")}
                className="w-full px-3 py-2 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] text-[14px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors"
              />
            )}
          </div>

          {/* ── Description ──────────────────────────────────── */}
          <div>
            <label className="block text-[13px] font-medium text-[var(--color-label-secondary)] mb-1.5">
              {t("ui.description", "Description")}
            </label>
            <textarea
              value={form.description}
              onChange={(e) => updateField("description", e.target.value)}
              placeholder={t("ui.a-brief-description-of-your-world-the-setting-to", "A brief description of your world — the setting, tone, and atmosphere...")}
              rows={3}
              className="w-full px-3 py-2 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] text-[14px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors resize-none"
            />
          </div>

          {/* ── System Prompt ────────────────────────────────── */}
          <div>
            <label className="block text-[13px] font-medium text-[var(--color-label-secondary)] mb-1.5">
              {t("ui.system-prompt", "System Prompt")} <span className="text-red-400">*</span>
            </label>
            <textarea
              value={form.system_prompt}
              onChange={(e) => updateField("system_prompt", e.target.value)}
              placeholder={`You are the narrator of an interactive story set in...\n\nDescribe the narrative style, character voice, world rules, and any constraints for the AI.`}
              rows={6}
              className={`w-full px-3 py-2 rounded-lg bg-[var(--color-fill-secondary)] border ${
                promptMissing && phase !== "idle"
                  ? "border-red-500/50"
                  : "border-[var(--color-separator)]"
              } text-[var(--color-label-primary)] text-[14px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors resize-none font-mono text-[13px]`}
            />
           <p className="text-[11px] text-[var(--color-label-tertiary)] mt-1">
              {t("ui.this-instructs-the-ai-on-how-to-narrate-your-sto", "This instructs the AI on how to narrate your story.")}
            </p>
          </div>

          {/* ── Privacy policy ─────────────────────────────────── */}
          <div className="pt-4 border-t border-[var(--color-separator)]/40">
            <h3 className="text-[13px] font-semibold text-[var(--color-label-primary)]">{t("ui.story-privacy", "Story privacy")}</h3>
            <p className="mt-1 mb-3 text-[11px] leading-relaxed text-[var(--color-label-tertiary)]">
              {t("ui.this-policy-controls-whether-story-text-may-be-s", "This policy controls whether story text may be sent to a cloud narrator. Local image generation remains local.")}
            </p>
            <div className="grid gap-2 sm:grid-cols-2">
              {[
                ["local_only", "Local only", "Never send this story to a cloud provider."],
                ["cloud_allowed", "Cloud allowed", "Cloud narration is available when selected."],
              ].map(([mode, label, description]) => (
                <label key={mode} className={`cursor-pointer rounded-lg border p-3 transition-colors focus-within:ring-2 focus-within:ring-[var(--color-accent)]/50 ${form.privacy_mode === mode ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10" : "border-[var(--color-separator)] bg-[var(--color-fill-secondary)]"}`}>
                  <input type="radio" name="privacy_mode" value={mode} checked={form.privacy_mode === mode} onChange={() => updateField("privacy_mode", mode as FormData["privacy_mode"])} className="sr-only" />
                  <span className="block text-[12px] font-medium text-[var(--color-label-primary)]">{label}</span>
                  <span className="mt-1 block text-[11px] text-[var(--color-label-tertiary)]">{description}</span>
                </label>
              ))}
            </div>
          </div>

          {/* ── NSFW toggle ──────────────────────────────────── */}
          {/* ── Visual Bible ──────────────────────────────────── */}
          <div className="pt-4 border-t border-[var(--color-separator)]/40 space-y-4">
            <div>
              <h3 className="text-[13px] font-semibold text-[var(--color-label-primary)]">{t("ui.visual-bible", "Visual Bible")}</h3>
              <p className="mt-1 text-[11px] leading-relaxed text-[var(--color-label-tertiary)]">{t("ui.keep-generated-scenes-visually-consistent-these", "Keep generated scenes visually consistent. These notes guide every illustration in this world.")}</p>
            </div>
            {([[
              "style", "Visual style", "e.g. cinematic anime, hand-painted manga, grounded realism"
            ], ["palette", "Color palette", "e.g. indigo shadows, warm amber light, muted crimson accents"], ["character_anchors", "Character anchors", "One line per character: appearance, hair, clothing, signature details"], ["location_anchors", "Location anchors", "Recurring locations, architecture, weather, and visual landmarks"], ["negative_prompt", "Always avoid", "e.g. modern clothing, text, watermark, extra fingers"]] as const).map(([key, label, placeholder]) => (
              <label key={key} className="block">
                <span className="mb-1.5 block text-[11px] font-medium text-[var(--color-label-secondary)]">{label}</span>
                {key === "style" || key === "palette" ? (
                  <input value={visualBible[key]} onChange={(event) => setVisualBible((current) => ({ ...current, [key]: event.target.value }))} placeholder={placeholder} className="w-full rounded-lg border border-[var(--color-separator)] bg-[var(--color-fill-secondary)] px-3 py-2 text-[12px] text-[var(--color-label-primary)] outline-none placeholder:text-[var(--color-label-tertiary)] focus:border-[var(--color-accent)]/50" />
                ) : (
                  <textarea value={visualBible[key]} onChange={(event) => setVisualBible((current) => ({ ...current, [key]: event.target.value }))} placeholder={placeholder} rows={2} className="w-full resize-y rounded-lg border border-[var(--color-separator)] bg-[var(--color-fill-secondary)] px-3 py-2 text-[12px] text-[var(--color-label-primary)] outline-none placeholder:text-[var(--color-label-tertiary)] focus:border-[var(--color-accent)]/50" />
                )}
              </label>
            ))}
          </div>

          {/* ── NSFW toggle ──────────────────────────────────── */}
          <label className="flex items-center gap-3 cursor-pointer group">
            <div className="relative">
              <input
                type="checkbox"
                checked={form.is_nsfw}
                onChange={(e) => updateField("is_nsfw", e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-9 h-5 rounded-full bg-[var(--color-fill-quaternary)] peer-checked:bg-red-500/60 transition-colors" />
              <div className="absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white peer-checked:translate-x-4 transition-transform shadow-sm" />
            </div>
            <span className="text-[13px] text-[var(--color-label-secondary)] group-hover:text-[var(--color-label-primary)] transition-colors">
              {t("ui.mature-content-18", "Mature content (18+)")}
            </span>
          </label>

          {/* ── Codex entries ─────────────────────────────────── */}
          <div className="pt-4 border-t border-[var(--color-separator)]/40">
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-[14px] font-semibold text-[var(--color-label-primary)]">
                {t("ui.codex-entries", "Codex Entries")}
              </h3>
              <button
                type="button"
                onClick={addCodexEntry}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-[var(--color-fill-tertiary)] hover:bg-[var(--color-fill-secondary)] text-[var(--color-label-primary)] text-[12px] font-medium transition-colors"
              >
                <Plus className="w-3.5 h-3.5" />
                {t("ui.add-entry", "Add entry")}
              </button>
            </div>
            <p className="text-[11px] text-[var(--color-label-tertiary)] mb-4">
              {t("ui.codex-entries-define-key-concepts-characters-loc", "Codex entries define key concepts, characters, locations, and\n              lore that the AI will reference during narration.")}
            </p>

            {codexEntries.length === 0 && (
              <button
                type="button"
                onClick={addCodexEntry}
                className="w-full text-center py-10 rounded-xl border-2 border-dashed border-[var(--color-separator)]/20 hover:border-[var(--color-accent)]/30 text-[var(--color-label-tertiary)] hover:text-[var(--color-accent)]/60 transition-colors group cursor-pointer"
              >
                <Plus className="w-6 h-6 mx-auto mb-2 opacity-40 group-hover:opacity-60 transition-opacity" />
                <p className="text-[12px]">
                  {t("ui.no-codex-entries-yet", "No codex entries yet")}
                </p>
                <p className="text-[11px] mt-1 opacity-60">
                  {t("ui.click-to-add-the-first-one", "Click to add the first one")}
                </p>
              </button>
            )}

            <div className="space-y-4">
              {codexEntries.map((entry, index) => (
                <div
                  key={entry.key}
                  className="rounded-xl border border-[var(--color-separator)]/30 bg-[var(--color-fill-quaternary)]/40 p-4 space-y-3"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] text-[var(--color-label-tertiary)] font-medium uppercase tracking-wider">
                      Entry {index + 1}
                    </span>
                    <button
                      type="button"
                      onClick={() => removeCodexEntry(entry.key)}
                      className="p-1 rounded text-[var(--color-label-tertiary)] hover:text-red-400 hover:bg-red-500/10 transition-colors"
                      aria-label={t("ui.remove-entry", "Remove entry")}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>

                  {/* Title */}
                  <div>
                    <label className="block text-[11px] text-[var(--color-label-tertiary)] mb-1 font-medium">
                      {t("ui.title", "Title")}
                    </label>
                    <input
                      type="text"
                      value={entry.title}
                      onChange={(e) =>
                        updateCodexEntry(entry.key, {
                          title: e.target.value,
                        })
                      }
                      placeholder="e.g. The Shadow Realm"
                      className="w-full px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] text-[13px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors"
                    />
                  </div>

                  {/* Content */}
                  <div>
                    <label className="block text-[11px] text-[var(--color-label-tertiary)] mb-1 font-medium">
                      {t("ui.content", "Content")}
                    </label>
                    <textarea
                      value={entry.content}
                      onChange={(e) =>
                        updateCodexEntry(entry.key, {
                          content: e.target.value,
                        })
                      }
                      placeholder={t("ui.describe-this-element-of-your-world", "Describe this element of your world...")}
                      rows={3}
                      className="w-full px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] text-[13px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors resize-none"
                    />
                  </div>

                  {/* Triggers */}
                  <div>
                    <label className="block text-[11px] text-[var(--color-label-tertiary)] mb-1 font-medium">
                      {t("ui.triggers", "Triggers")}
                    </label>
                    <input
                      type="text"
                      value={entry.triggers}
                      onChange={(e) =>
                        updateCodexEntry(entry.key, {
                          triggers: e.target.value,
                        })
                      }
                      placeholder="shadow realm, dark dimension, void"
                      className="w-full px-3 py-1.5 rounded-lg bg-[var(--color-fill-secondary)] border border-[var(--color-separator)] text-[var(--color-label-primary)] text-[13px] placeholder:text-[var(--color-label-tertiary)] outline-none focus:border-[var(--color-accent)]/50 transition-colors"
                    />
                    <p className="text-[10px] text-[var(--color-label-tertiary)] mt-0.5">
                      {t("ui.comma-separated-keywords-that-trigger-this-codex", "Comma-separated keywords that trigger this codex entry.")}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* ── Submit ──────────────────────────────────────────── */}
          <div className="pt-4 border-t border-[var(--color-separator)]/40 flex justify-end gap-3">
            <button
              onClick={cancelEditing}
              className="px-5 py-2.5 rounded-lg bg-[var(--color-fill-tertiary)] hover:bg-[var(--color-fill-secondary)] text-[var(--color-label-primary)] text-[13px] font-medium transition-colors"
            >
              {t("ui.cancel", "Cancel")}
            </button>
            <button
              onClick={handleSubmit}
              disabled={!canSubmit || isSubmitting}
              className="inline-flex items-center gap-2 px-6 py-2.5 rounded-lg bg-[var(--color-accent)] hover:bg-[color-mix(in_srgb,var(--color-accent)_80%,white)] disabled:bg-[var(--color-fill-tertiary)] text-black disabled:text-[var(--color-label-tertiary)] text-[13px] font-semibold transition-colors disabled:cursor-not-allowed"
            >
              {isSubmitting ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {phase === "creating_world"
                    ? "Creating world..."
                    : "Saving codex entries..."}
                </>
              ) : (
                <>
                  <Globe className="w-4 h-4" />
                  {t("ui.create-story", "Create Story")}
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
