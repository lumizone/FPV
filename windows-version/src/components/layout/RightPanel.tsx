import { useCallback, useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useApp } from "@/lib/store";
import { BookMarked, BookOpen, GitBranch, Map as MapIcon, Network, Pin, Plus, Trash2, Users, X } from "lucide-react";
import { OrnamentDivider, SectionHeading } from "@/components/ui/ornament";
import { StoryMap } from "@/components/narration/StoryMap";
import { sessionForkFromMessage, storyMemoryList, storyPinnedCanonAdd, storyPinnedCanonDelete, storyPinnedCanonList, storyStateGet, storyStateSet, type CodexEntry, type Character, type PinnedCanon, type SessionSummary, type StoryMemoryRecord, type StoryState, type PendingChange, type Quest, type InventoryItem, type RelationshipRecord } from "@/lib/tauri";
import { waitForContinuity } from "@/lib/narration/generate";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";

function buildSessionTree(sessions: SessionSummary[]) {
  const byParent = new Map<string | null, SessionSummary[]>();
  for (const session of sessions) {
    const siblings = byParent.get(session.parent_session_id) ?? [];
    siblings.push(session);
    byParent.set(session.parent_session_id, siblings);
  }
  for (const siblings of byParent.values()) {
    siblings.sort((a, b) => a.created_at.localeCompare(b.created_at));
  }
  return byParent;
}

/**
 * RightPanel — contextual world information. Auto-opens during gameplay
 * (session view), auto-closes on library/home so it never shows garbage.
 */
export function RightPanel() {
  const { t } = useTranslation();
  const activeView = useApp((s) => s.active_view);
  const currentWorldId = useApp((s) => s.current_world_id);
  const selectedWorldForDetail = useApp((s) => s.selected_world_for_detail);
  const worlds = useApp((s) => s.worlds);
  const setRightPanelOpen = useApp((s) => s.setRightPanelOpen);
  const workspaceMessages = useApp((s) => s.workspace_messages);
  const workspaceSceneImages = useApp((s) => s.workspace_scene_images);
  const workspaceStoryState = useApp((s) => s.workspace_story_state);
  const requestWorkspaceJump = useApp((s) => s.requestWorkspaceJump);
  const currentSessionId = useApp((s) => s.current_session_id);
  const [tool, setTool] = useState<"overview" | "codex" | "map" | "state" | "memory" | "paths">("overview");
  const [storyStateSaving, setStoryStateSaving] = useState(false);
  const [pinnedCanon, setPinnedCanon] = useState<PinnedCanon[]>([]);
  const [memoryRecords, setMemoryRecords] = useState<StoryMemoryRecord[]>([]);
  const [sessionSummary, setSessionSummary] = useState("");
  const [sessionPaths, setSessionPaths] = useState<SessionSummary[]>([]);
  const [newCanon, setNewCanon] = useState("");
  const [memoryError, setMemoryError] = useState<string | null>(null);

  // Right panel is ONLY available during active story narration.
  // Auto-close on every view except session — it must never show
  // stale data from a previous world while browsing the library.
  useEffect(() => {
    if (activeView === "session") {
      setRightPanelOpen(true);
    } else {
      setRightPanelOpen(false);
    }
  }, [activeView, setRightPanelOpen]);

  // Derive world to show — prefer active session's world, then detail selection
  const worldId = currentWorldId || selectedWorldForDetail?.id || null;
  const world = worlds.find((w) => w.id === worldId) || selectedWorldForDetail || null;

  const createBranch = async () => {
    const sessionId = currentSessionId;
    if (!sessionId || !worldId) return;
    const label = window.prompt(t("ui.name-this-story-branch", "Name this story branch"), t("ui.alternative-path", "Alternative path"))?.trim();
    if (!label) return;
    try {
      const branchId = await invoke<string>("session_branch", { session_id: sessionId, label });
      useApp.getState().setActiveSession(branchId, worldId);
    } catch (error) {
      console.error("Could not create story branch", error);
      toast.error(t("ui.could-not-create-story-branch", "Could not create story branch"), { description: String(error) });
    }
  };

  const createCheckpoint = async () => {
    if (!currentSessionId) return;
    const label = window.prompt(t("ui.name-this-checkpoint", "Name this checkpoint"), t("ui.before-the-next-choice", "Before the next choice"))?.trim();
    if (!label) return;
    try {
      await invoke<string>("session_checkpoint", { session_id: currentSessionId, label });
      if (worldId) {
        const paths = await invoke<SessionSummary[]>("session_list_for_world", { world_id: worldId });
        setSessionPaths(paths);
      }
    } catch (error) {
      console.error("Could not create checkpoint", error);
      toast.error(t("ui.could-not-create-checkpoint", "Could not create checkpoint"), { description: String(error) });
    }
  };

  const forkFromMessage = async (messageId: string) => {
    if (!currentSessionId || !worldId) return;
    const label = window.prompt(t("ui.name-this-alternate-path", "Name this alternate path"), t("ui.retcon", "Retcon"))?.trim();
    if (!label) return;
    try {
      const sessionId = await sessionForkFromMessage(currentSessionId, messageId, label);
      useApp.getState().setActiveSession(sessionId, worldId);
    } catch (error) {
      console.error("Could not fork from this turn", error);
      toast.error(t("ui.could-not-fork-from-this-turn", "Could not fork from this turn"), { description: String(error) });
    }
  };

  const updateWorkspaceState = (patch: Partial<StoryState>) => {
    if (workspaceStoryState) {
      useApp.getState().setWorkspaceStoryState({ ...workspaceStoryState, ...patch });
    }
  };

  const saveWorkspaceState = async () => {
    const sessionId = useApp.getState().current_session_id;
    if (!sessionId || !workspaceStoryState) return;
    setStoryStateSaving(true);
    const previousState = workspaceStoryState;
    try {
      const nextState = { ...workspaceStoryState, source: "manual" as const };
      useApp.getState().setWorkspaceStoryState(nextState);
      await storyStateSet(sessionId, nextState);
    } catch (error) {
      useApp.getState().setWorkspaceStoryState(previousState);
      setMemoryError(error instanceof Error ? error.message : "Story state could not be saved.");
    } finally {
      setStoryStateSaving(false);
    }
  };

  const resolvePendingChange = async (change: PendingChange, accept: boolean) => {
    if (!workspaceStoryState || !currentSessionId) return;
    let persistedState: StoryState;
    try {
      await waitForContinuity(currentSessionId);
      persistedState = await storyStateGet(currentSessionId);
    } catch (error) {
      setMemoryError(error instanceof Error ? error.message : "Story state could not be loaded.");
      return;
    }
    let nextState = { ...persistedState, pending_changes: (persistedState.pending_changes ?? []).filter((item) => item.id !== change.id), source: "manual" };
    if (accept) {
      try {
        const value = change.value ? JSON.parse(change.value) : {};
        if (change.kind === "quest_add") {
          const quest = value as Quest;
          nextState.quests = [...(nextState.quests ?? []).filter((item) => item.id !== quest.id), quest].slice(-8);
        } else if (change.kind === "quest_update") {
          nextState.quests = (nextState.quests ?? []).map((quest) => quest.id === change.target ? { ...quest, ...value } : quest);
        } else if (change.kind === "inventory_add") {
          const item = value as InventoryItem;
          nextState.inventory_items = [...(nextState.inventory_items ?? []).filter((current) => current.name !== item.name), item].slice(-12);
        } else if (change.kind === "inventory_remove") {
          nextState.inventory_items = (nextState.inventory_items ?? []).filter((item) => item.name !== change.target);
        } else if (change.kind === "relationship_update") {
          const record = value as RelationshipRecord;
          nextState.relationship_records = [...(nextState.relationship_records ?? []).filter((current) => current.character !== record.character), record].slice(-12);
        }
      } catch {
        setMemoryError("This proposed change could not be read. It was kept out of the story state.");
        return;
      }
    }
    useApp.getState().setWorkspaceStoryState(nextState);
    try {
      await storyStateSet(currentSessionId, nextState);
    } catch (error) {
      useApp.getState().setWorkspaceStoryState(persistedState);
      setMemoryError(error instanceof Error ? error.message : "Could not save the state change.");
    }
  };

  const [codex, setCodex] = useState<CodexEntry[] | null>(null);
  const [characters, setCharacters] = useState<Character[] | null>(null);
  const [imgError, setImgError] = useState(false);

  useEffect(() => {
    if (!currentSessionId) {
      setPinnedCanon([]);
      setMemoryRecords([]);
      setSessionSummary("");
      return;
    }
    let cancelled = false;
    setMemoryError(null);
    Promise.all([
      storyPinnedCanonList(currentSessionId),
      storyMemoryList(currentSessionId),
      invoke<string>("session_get_summary", { session_id: currentSessionId }),
    ]).then(([canon, records, summary]) => {
      if (cancelled) return;
      setPinnedCanon(canon);
      setMemoryRecords(records);
      setSessionSummary(summary);
    }).catch((error) => {
      if (!cancelled) setMemoryError(error instanceof Error ? error.message : "Memory could not be loaded.");
    });
    return () => { cancelled = true; };
  }, [currentSessionId]);

  const loadPaths = useCallback(async () => {
    if (!worldId) {
      setSessionPaths([]);
      return;
    }
    try {
      const paths = await invoke<SessionSummary[]>("session_list_for_world", { world_id: worldId });
      setSessionPaths(paths);
    } catch {
      setSessionPaths([]);
    }
  }, [worldId]);

  useEffect(() => {
    void loadPaths();
  }, [loadPaths, currentSessionId]);

  const addPinnedCanon = async () => {
    if (!currentSessionId || !newCanon.trim()) return;
    try {
      await storyPinnedCanonAdd(currentSessionId, newCanon.trim());
      setPinnedCanon(await storyPinnedCanonList(currentSessionId));
      setNewCanon("");
    } catch (error) {
      setMemoryError(error instanceof Error ? error.message : "Could not pin this fact.");
    }
  };

  const removePinnedCanon = async (id: string) => {
    try {
      await storyPinnedCanonDelete(id);
      setPinnedCanon((current) => current.filter((item) => item.id !== id));
    } catch (error) {
      setMemoryError(error instanceof Error ? error.message : "Could not remove this fact.");
    }
  };

  const renderPathTree = (byParent: Map<string | null, SessionSummary[]>, parentId: string | null, depth = 0): ReactNode[] =>
    (byParent.get(parentId) ?? []).flatMap((path) => [
      <div key={path.id} className="relative" style={{ marginLeft: depth * 14 }}>
        {depth > 0 && <span aria-hidden="true" className="absolute -left-3 top-0 h-5 w-3 border-l border-b border-[var(--color-separator)]/30" />}
        <div className={`flex items-center gap-1.5 rounded-lg border px-2 py-2 ${path.id === currentSessionId ? "border-[var(--color-accent)]/50 bg-[var(--color-accent-soft)]" : "border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30"}`}>
          <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${path.id === currentSessionId ? "bg-[var(--color-accent)]" : "bg-[var(--color-label-quaternary)]"}`} />
          <button onClick={() => { if (worldId) useApp.getState().setActiveSession(path.id, worldId); }} className="min-w-0 flex-1 text-left" aria-current={path.id === currentSessionId ? "page" : undefined}>
            <span className="block truncate text-[11px] text-[var(--color-label-primary)]">{path.branch_label || "Main timeline"}</span>
            <span className="mt-0.5 block text-[9px] text-[var(--color-label-quaternary)]">{path.message_count} messages{path.is_checkpoint ? " · save point" : ""}</span>
          </button>
          {path.id !== currentSessionId && <button onClick={() => { if (worldId) useApp.getState().setActiveSession(path.id, worldId); }} className="shrink-0 text-[9px] text-[var(--color-accent)] hover:underline" aria-label={t("ui.open-path", "Open {{name}}", { name: path.branch_label || t("ui.main-timeline", "main timeline") })}>{t("ui.open", "Open")}</button>}
          {path.id !== currentSessionId && <button onClick={() => { if (window.confirm(t("ui.delete-session-confirm", "Delete this adventure? This cannot be undone."))) { invoke("session_delete", { session_id: path.id }).then(() => loadPaths()).catch(() => {}); } }} className="shrink-0 p-1 text-[var(--color-label-quaternary)] hover:text-red-400 transition-colors" aria-label={t("ui.delete-session", "Delete session")} title={t("ui.delete-session", "Delete session")}><Trash2 className="h-3 w-3" /></button>}
        </div>
      </div>,
      ...renderPathTree(byParent, path.id, depth + 1),
    ]);

  // Load codex and characters when world changes
  useEffect(() => {
    if (!world) {
      setCodex(null);
      setCharacters(null);
      return;
    }
    let cancelled = false;
    invoke<CodexEntry[]>("codex_list_for_world", { world_id: world.id })
      .then((entries) => { if (!cancelled) setCodex(entries); })
      .catch(() => { if (!cancelled) setCodex([]); });
    invoke<Character[]>("character_list", { world_id: world.id })
      .then((list) => { if (!cancelled) setCharacters(list); })
      .catch(() => { if (!cancelled) setCharacters([]); });
    return () => { cancelled = true; };
  }, [world?.id]);

  // Empty state — no world selected
  if (!world) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-center px-4">
        <div className="text-[28px] text-[var(--color-label-quaternary)] mb-3">✦</div>
        <p className="text-[12px] text-[var(--color-label-tertiary)] leading-relaxed">
          {t("ui.select-a-world-to-see-its-lore-characters-and-de", "Select a world to see its lore, characters, and details.")}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* ── Header ─────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-3 py-2.5 border-b border-[var(--color-separator)]/20 shrink-0">
        <h3 className="text-[11px] font-display tracking-[0.04em] text-[var(--color-label-primary)] truncate">
          {world.name}
        </h3>
        <button
          onClick={() => setRightPanelOpen(false)}
          className="p-1 rounded-md text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] transition-colors"
          aria-label={t("ui.close-panel", "Close panel")}
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="grid grid-cols-3 gap-1 px-2 py-2 border-b border-[var(--color-separator)]/20 shrink-0">
        {([
          ["overview", "World", Users],
          ["codex", "Codex", BookOpen],
            ["map", "Map", MapIcon],
            ["state", "State", Network],
            ["memory", "Memory", BookMarked],
            ["paths", "Paths", GitBranch],
        ] as const).map(([id, label, Icon]) => (
          <button
            key={id}
            onClick={() => setTool(id)}
            className={`flex flex-col items-center gap-1 rounded-lg px-1 py-1.5 text-[9px] transition-colors ${tool === id ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]" : "text-[var(--color-label-tertiary)] hover:text-[var(--color-label-primary)]"}`}
            aria-label={label}
            aria-pressed={tool === id}
          >
            <Icon className="w-3.5 h-3.5" />
            {label}
          </button>
        ))}
      </div>
      <div className="grid grid-cols-2 gap-1.5 mx-3 my-2">
        <button onClick={createCheckpoint} className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-[var(--color-separator)]/25 px-2 py-1.5 text-[10px] text-[var(--color-label-secondary)] hover:border-[var(--color-accent)]/40 hover:text-[var(--color-accent)] transition-colors" aria-label={t("ui.save-a-named-checkpoint", "Save a named checkpoint")}>
          <Pin className="w-3 h-3" /> {t("ui.save-point", "Save point")}
        </button>
        <button onClick={createBranch} className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-[var(--color-separator)]/25 px-2 py-1.5 text-[10px] text-[var(--color-label-secondary)] hover:border-[var(--color-accent)]/40 hover:text-[var(--color-accent)] transition-colors" aria-label={t("ui.create-a-new-story-branch", "Create a new story branch")}>
          <GitBranch className="w-3 h-3" /> {t("ui.new-path", "New path")}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {tool === "map" && (
          <StoryMap
            open
            embedded
            onClose={() => setTool("overview")}
            messages={workspaceMessages}
            sceneImages={workspaceSceneImages}
            onJumpToMessage={requestWorkspaceJump}
            onForkFromMessage={forkFromMessage}
          />
        )}
        {tool === "state" && (
          <div className="p-3">
            {!workspaceStoryState ? (
              <p className="text-[11px] text-[var(--color-label-tertiary)]">{t("ui.story-state-is-not-available-yet", "Story state is not available yet.")}</p>
            ) : (
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <div>
                    <h3 className="text-[12px] font-semibold text-[var(--color-label-primary)]">{t("ui.story-state", "Story State")}</h3>
                    <p className="text-[10px] text-[var(--color-label-tertiary)]">{t("ui.continuity-facts-for-this-story", "Continuity facts for this story.")}</p>
                  </div>
                  <button onClick={saveWorkspaceState} disabled={storyStateSaving} className="text-[10px] text-[var(--color-accent)] disabled:opacity-50">
                    {storyStateSaving ? "Saving..." : "Save"}
                  </button>
                </div>
                {workspaceStoryState.conflicts.length > 0 && (
                  <div className="rounded-lg border border-[var(--color-warm)]/30 bg-[var(--color-warm)]/10 px-2.5 py-2 text-[10px] text-[var(--color-warm)]">
                    <strong>{t("ui.review-continuity", "Review continuity")}</strong>
                    <ul className="mt-1 list-disc pl-4 text-[var(--color-label-secondary)]">
                      {workspaceStoryState.conflicts.map((conflict) => <li key={conflict}>{conflict}</li>)}
                    </ul>
                  </div>
                )}
                {(workspaceStoryState.pending_changes?.length ?? 0) > 0 && <section className="rounded-lg border border-[var(--color-accent)]/25 bg-[var(--color-accent-soft)]/30 p-2.5"><div className="mb-2"><h4 className="text-[11px] font-semibold text-[var(--color-label-primary)]">{t("ui.proposed-changes", "Proposed changes")}</h4><p className="mt-0.5 text-[10px] text-[var(--color-label-tertiary)]">{t("ui.the-narrator-noticed-these-possible-changes-acce", "The narrator noticed these possible changes. Accept only what should become canon.")}</p></div><div className="space-y-2">{workspaceStoryState.pending_changes?.map((change) => <div key={change.id} className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/40 p-2"><p className="text-[10px] leading-relaxed text-[var(--color-label-secondary)]">{change.summary}</p><div className="mt-1.5 flex gap-1.5"><button onClick={() => void resolvePendingChange(change, true)} className="rounded-md bg-[var(--color-accent)] px-2 py-1 text-[9px] font-medium text-black">{t("ui.accept", "Accept")}</button><button onClick={() => void resolvePendingChange(change, false)} className="rounded-md border border-[var(--color-separator)]/30 px-2 py-1 text-[9px] text-[var(--color-label-tertiary)]">{t("ui.reject", "Reject")}</button></div></div>)}</div></section>}
                <label className="block text-[10px] text-[var(--color-label-tertiary)]">
                  {t("ui.current-location", "Current location")}
                  <input value={workspaceStoryState.location} onChange={(event) => updateWorkspaceState({ location: event.target.value })} className="mt-1 w-full rounded-lg bg-[var(--color-fill-tertiary)] px-2 py-1.5 text-[11px] text-[var(--color-label-primary)] outline-none" />
                </label>
                {([["active_goals", "Active goals"], ["open_threads", "Open threads"], ["inventory", "Inventory"], ["relationships", "Relationships"], ["facts", "Known facts"]] as const).map(([field, label]) => (
                  <label key={field} className="block text-[10px] text-[var(--color-label-tertiary)]">
                    {label}
                    <textarea value={workspaceStoryState[field].join("\n")} onChange={(event) => updateWorkspaceState({ [field]: event.target.value.split("\n").map((value) => value.trim()).filter(Boolean) })} rows={2} className="mt-1 w-full resize-y rounded-lg bg-[var(--color-fill-tertiary)] px-2 py-1.5 text-[11px] text-[var(--color-label-primary)] outline-none" />
                  </label>
                ))}
                {(workspaceStoryState.quests?.length ?? 0) > 0 && <section className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/20 p-2"><h4 className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.quest-log", "Quest log")}</h4><div className="space-y-1">{workspaceStoryState.quests?.map((quest) => <div key={quest.id} className="flex items-start gap-1.5 text-[10px]"><span className={`mt-1 h-1.5 w-1.5 rounded-full ${quest.status === "completed" ? "bg-emerald-400" : quest.status === "failed" ? "bg-red-400" : "bg-[var(--color-accent)]"}`} /><span className="text-[var(--color-label-secondary)]"><strong className="text-[var(--color-label-primary)]">{quest.title}</strong> <span className="text-[var(--color-label-quaternary)]">{quest.status}</span>{quest.description && <span className="block text-[var(--color-label-tertiary)]">{quest.description}</span>}</span></div>)}</div></section>}
                {(workspaceStoryState.inventory_items?.length ?? 0) > 0 && <section className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/20 p-2"><h4 className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.inventory", "Inventory")}</h4><div className="flex flex-wrap gap-1">{workspaceStoryState.inventory_items?.map((item) => <span key={item.name} className="rounded-full bg-[var(--color-fill-tertiary)] px-1.5 py-0.5 text-[9px] text-[var(--color-label-secondary)]">{item.name} x{item.quantity}</span>)}</div></section>}
                {(workspaceStoryState.relationship_records?.length ?? 0) > 0 && <section className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/20 p-2"><h4 className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.relationships", "Relationships")}</h4><div className="space-y-1">{workspaceStoryState.relationship_records?.map((record) => <div key={record.character} className="flex items-center justify-between text-[10px]"><span className="text-[var(--color-label-secondary)]">{record.character}</span><span className="text-[var(--color-accent)]">{record.status}</span></div>)}</div></section>}
              </div>
            )}
          </div>
        )}
        {tool === "memory" && (
          <div className="p-3 space-y-4">
            <div>
              <h3 className="text-[12px] font-semibold text-[var(--color-label-primary)]">{t("ui.what-the-narrator-knows", "What the narrator knows")}</h3>
              <p className="mt-0.5 text-[10px] leading-relaxed text-[var(--color-label-tertiary)]">{t("ui.pinned-canon-is-always-included-story-state-summ", "Pinned canon is always included. Story State, summary, recent turns, and relevant past scenes supply the rest of each prompt.")}</p>
            </div>
            {memoryError && <p role="alert" className="rounded-lg bg-red-500/10 px-2 py-1.5 text-[10px] text-red-300">{memoryError}</p>}
            <section>
              <div className="mb-1.5 flex items-center justify-between"><h4 className="text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.pinned-canon", "Pinned canon")}</h4><span className="text-[9px] text-[var(--color-label-quaternary)]">{t("ui.always-remembered", "Always remembered")}</span></div>
              <div className="space-y-1.5">
                {pinnedCanon.map((item) => <div key={item.id} className="flex gap-1 rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 px-2 py-1.5"><p className="min-w-0 flex-1 text-[11px] leading-relaxed text-[var(--color-label-secondary)]">{item.content}</p><button onClick={() => void removePinnedCanon(item.id)} className="shrink-0 self-start p-0.5 text-[var(--color-label-quaternary)] hover:text-red-300" aria-label={t("ui.remove-pinned-canon", "Remove pinned canon")}><Trash2 className="h-3 w-3" /></button></div>)}
                {pinnedCanon.length === 0 && <p className="text-[10px] italic text-[var(--color-label-quaternary)]">{t("ui.pin-a-fact-when-it-must-never-be-lost", "Pin a fact when it must never be lost.")}</p>}
              </div>
              <div className="mt-2 flex gap-1"><input value={newCanon} onChange={(event) => setNewCanon(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void addPinnedCanon(); }} placeholder="e.g. Mara cannot swim" className="min-w-0 flex-1 rounded-lg bg-[var(--color-fill-tertiary)] px-2 py-1.5 text-[11px] text-[var(--color-label-primary)] outline-none" aria-label={t("ui.fact-to-pin-as-canon", "Fact to pin as canon")} /><button onClick={() => void addPinnedCanon()} disabled={!newCanon.trim()} className="rounded-lg bg-[var(--color-accent-soft)] px-2 text-[var(--color-accent)] disabled:opacity-40" aria-label={t("ui.pin-fact", "Pin fact")}><Plus className="h-3.5 w-3.5" /></button></div>
            </section>
            <section><h4 className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.story-so-far", "Story so far")}</h4><p className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 px-2 py-1.5 text-[11px] leading-relaxed text-[var(--color-label-secondary)]">{sessionSummary || "A rolling summary will appear once the story has enough turns."}</p></section>
            <section><div className="mb-1.5 flex items-center justify-between"><h4 className="text-[10px] font-medium uppercase tracking-wide text-[var(--color-label-tertiary)]">{t("ui.indexed-past-scenes", "Indexed past scenes")}</h4><span className="text-[9px] text-[var(--color-label-quaternary)]">{t("ui.used-only-when-relevant", "Used only when relevant")}</span></div><div className="space-y-1.5">{memoryRecords.length === 0 ? <p className="text-[10px] italic text-[var(--color-label-quaternary)]">{t("ui.semantic-recall-is-empty-or-turned-off", "Semantic recall is empty or turned off.")}</p> : memoryRecords.map((record) => <p key={record.message_id} className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 px-2 py-1.5 text-[10px] leading-relaxed text-[var(--color-label-secondary)]">{record.content.slice(0, 240)}{record.content.length > 240 ? "..." : ""}</p>)}</div></section>
          </div>
        )}
        {tool === "paths" && (
          <div className="p-3 space-y-3"><div><h3 className="text-[12px] font-semibold text-[var(--color-label-primary)]">{t("ui.story-paths", "Story paths")}</h3><p className="mt-0.5 text-[10px] text-[var(--color-label-tertiary)]">{t("ui.every-save-point-and-alternate-choice-stays-conn", "Every save point and alternate choice stays connected to the moment it came from.")}</p></div>{sessionPaths.length === 0 ? <p className="text-[10px] italic text-[var(--color-label-quaternary)]">{t("ui.no-saved-paths-yet", "No saved paths yet.")}</p> : <div className="space-y-1.5">{renderPathTree(buildSessionTree(sessionPaths), null)}</div>}</div>
        )}
        {tool === "codex" && (
          <div className="p-3 space-y-2">
            {codex === null ? (
              <p className="text-[10px] text-[var(--color-label-tertiary)] italic">{t("ui.loading-lore", "Loading lore...")}</p>
            ) : codex.length === 0 ? (
              <p className="text-[10px] text-[var(--color-label-tertiary)] italic">{t("ui.no-lore-entries-for-this-world", "No lore entries for this world.")}</p>
            ) : codex.map((entry) => (
              <details key={entry.id} className="rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30">
                <summary className="cursor-pointer px-2.5 py-2 text-[11px] font-medium text-[var(--color-label-primary)]">{entry.title}</summary>
                <p className="px-2.5 pb-2 text-[11px] font-serif leading-relaxed text-[var(--color-label-secondary)] whitespace-pre-wrap">{entry.content}</p>
              </details>
            ))}
          </div>
        )}
        {tool === "overview" && (<>
        {/* ── Cover image ───────────────────────────────────────── */}
        {world.cover_image_path && !imgError && (
          <div className="aspect-[16/9] overflow-hidden bg-[var(--color-fill-tertiary)]">
            <img
              src={convertFileSrc(world.cover_image_path)}
              alt={`${world.name} cover`}
              className="w-full h-full object-cover"
              loading="lazy"
              onError={() => setImgError(true)}
            />
          </div>
         )}

        {/* ── Genre badge ───────────────────────────────────────── */}
        <div className="px-3 pt-3">
          <span className="inline-block text-[10px] uppercase tracking-[0.06em] font-medium text-[var(--color-accent)] bg-[var(--color-accent-soft)] px-2 py-0.5 rounded-full">
            {world.genre}
          </span>
        </div>

        {/* ── Description ───────────────────────────────────────── */}
        {world.description && (
          <p className="px-3 pt-2 text-[12px] font-serif leading-relaxed text-[var(--color-label-secondary)]">
            {world.description}
          </p>
        )}

        <OrnamentDivider className="px-3 py-3" />

        {/* ── Characters ────────────────────────────────────────── */}
        <div className="px-3 pb-2">
          <SectionHeading
            icon={<Users className="w-3 h-3" />}
            label="Characters"
            count={characters !== null ? characters.length : undefined}
          />

          {characters === null ? (
            <p className="text-[10px] text-[var(--color-label-quaternary)] italic">{t("ui.loading", "Loading...")}</p>
          ) : characters.length === 0 ? (
            <p className="text-[10px] text-[var(--color-label-quaternary)] italic">
              {t("ui.no-characters-defined-for-this-world", "No characters defined for this world.")}
            </p>
          ) : (
            <div className="space-y-1">
              {characters.map((ch) => (
                <details key={ch.id} className="group rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 overflow-hidden">
                  <summary className="flex items-center gap-2 px-2.5 py-1.5 cursor-pointer text-[11px] text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)]/50 transition-colors list-none">
                    <span className="w-5 h-5 rounded-full bg-[var(--color-accent-soft)] flex items-center justify-center text-[9px] font-semibold text-[var(--color-accent)] shrink-0">
                      {ch.name.slice(0, 2).toUpperCase()}
                    </span>
                    <span className="truncate">{ch.name}</span>
                    <span className={`ml-auto text-[9px] px-1.5 py-0.5 rounded-full shrink-0 ${
                      ch.role === "player" ? "bg-blue-500/15 text-blue-400" :
                      ch.role === "narrator" ? "bg-amber-500/15 text-amber-400" :
                      "bg-purple-500/15 text-purple-400"
                    }`}>
                      {ch.role}
                    </span>
                  </summary>
                  <div className="px-2.5 pb-2 pt-0.5">
                    {ch.backstory && (
                      <p className="text-[11px] font-serif leading-relaxed text-[var(--color-label-secondary)]">
                        {ch.backstory.slice(0, 200)}
                        {ch.backstory.length > 200 ? "…" : ""}
                      </p>
                    )}
                    {ch.traits && ch.traits !== "[]" && (
                      <div className="flex flex-wrap gap-1 mt-1.5">
                        {(() => {
                          try {
                            const t: string[] = JSON.parse(ch.traits);
                            return t.map((trait, i) => (
                              <span key={i} className="text-[9px] px-1.5 py-0.5 rounded-full bg-[var(--color-fill-quaternary)] text-[var(--color-label-secondary)]">
                                {trait}
                              </span>
                            ));
                          } catch { return null; }
                        })()}
                      </div>
                    )}
                  </div>
                </details>
              ))}
            </div>
          )}
        </div>

        {/* ── Codex ─────────────────────────────────────────────── */}
        <div className="px-3 pb-4">
          <SectionHeading
            icon={<BookOpen className="w-3 h-3" />}
            label="Codex"
            count={codex !== null ? codex.length : undefined}
          />

          {codex === null ? (
            <p className="text-[10px] text-[var(--color-label-quaternary)] italic">{t("ui.loading", "Loading...")}</p>
          ) : codex.length === 0 ? (
            <p className="text-[10px] text-[var(--color-label-quaternary)] italic">
              {t("ui.no-lore-entries-for-this-world", "No lore entries for this world.")}
            </p>
          ) : (
            <div className="space-y-1">
              {codex.slice(0, 10).map((entry) => (
                <details key={entry.id} className="group rounded-lg border border-[var(--color-separator)]/20 bg-[var(--color-fill-quaternary)]/30 overflow-hidden">
                  <summary className="flex items-center justify-between px-2.5 py-1.5 cursor-pointer text-[11px] font-medium text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)]/50 transition-colors list-none">
                    <span className="font-display tracking-[0.02em] truncate pr-2">
                      {entry.title}
                    </span>
                    {entry.triggers && entry.triggers !== "[]" && (
                      <span className="text-[8px] text-[var(--color-label-tertiary)] font-mono shrink-0 opacity-60">
                        {(() => {
                          try {
                            const t: string[] = JSON.parse(entry.triggers);
                            return t.slice(0, 2).join(", ");
                          } catch { return ""; }
                        })()}
                      </span>
                    )}
                  </summary>
                  <div className="px-2.5 pb-2 pt-0.5">
                    <p className="text-[11px] font-serif leading-relaxed text-[var(--color-label-secondary)] whitespace-pre-wrap">
                      {entry.content.slice(0, 300)}
                      {entry.content.length > 300 ? "…" : ""}
                    </p>
                  </div>
                </details>
              ))}
              {codex.length > 10 && (
                <p className="text-[10px] text-[var(--color-label-quaternary)] text-center py-1">
                  +{codex.length - 10} more entries
                </p>
              )}
            </div>
          )}
        </div>
        </>)}
      </div>
    </div>
  );
}
