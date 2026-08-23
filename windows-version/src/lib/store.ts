/**
 * FPV2.0 global app state — stripped of all Local Waifu companion-AI state.
 * Only FPV story narration state is kept.
 */
import { create } from "zustand";
import { invoke, Channel } from "@tauri-apps/api/core";
import { toast } from "sonner";
import i18n from "@/i18n/config";
import type {
  HardwareInfo,
  PullProgress,
  World,
  Character,
  StoryState,
} from "./tauri";
import { characterList, modelPull, modelPullCancel, modelSetDefault, settingSet } from "./tauri";

export interface WorkspaceMessage {
  id: string;
  role: "narrator" | "user";
  content: string;
  created_at: string;
}

export interface ModelDownloadState {
  id: string;
  role: "chat" | "embed";
  status: string;
  completed: number;
  total: number;
  phase: "active" | "done" | "error" | "cancelled";
  error?: string;
}

interface AppState {
  // hardware (cached at startup)
  hardware: HardwareInfo | null;
  setHardware: (h: HardwareInfo | null) => void;

  // story state (FPV)
  active_view: "home" | "library" | "session" | "world_new" | "world_detail";
  setActiveView: (v: AppState["active_view"]) => void;

  current_session_id: string | null;
  current_world_id: string | null;
  setActiveSession: (sessionId: string, worldId: string) => void;
  clearRuntimeState: () => void;

  // World library
  worlds: World[];
  setWorlds: (w: World[]) => void;
  refreshWorlds: () => Promise<void>;

  // world detail view
  selected_world_for_detail: World | null;
  setSelectedWorldForDetail: (w: World | null) => void;

  // characters for current world detail view
  characters: Character[];
  setCharacters: (c: Character[]) => void;
  refreshCharacters: (worldId: string) => Promise<void>;

  // world edit mode (null = create new, string = edit existing)
  editing_world_id: string | null;
  setEditingWorldId: (id: string | null) => void;

  // panel layout state
  left_panel_open: boolean;
  setLeftPanelOpen: (b: boolean) => void;
  right_panel_open: boolean;
  setRightPanelOpen: (b: boolean) => void;

  // Active narration tools mirrored into the contextual right panel
  workspace_messages: WorkspaceMessage[];
  workspace_scene_images: Record<number, string>;
  workspace_story_state: StoryState | null;
  workspace_jump_to_message: number | null;
  setWorkspaceMessages: (messages: WorkspaceMessage[]) => void;
  setWorkspaceSceneImages: (images: Record<number, string>) => void;
  setWorkspaceStoryState: (state: StoryState | null) => void;
  requestWorkspaceJump: (index: number) => void;
  clearWorkspaceJump: () => void;

  // settings drawer
  settings_open: boolean;
  setSettingsOpen: (b: boolean) => void;
  settings_tab: "general" | "appearance" | "narrative_model" | "image_model" | "byok" | "about";
  setSettingsTab: (t: AppState["settings_tab"]) => void;

  // model selector
  selected_model: string | null;
  setSelectedModel: (m: string | null) => void;

  // model download
  model_download: ModelDownloadState | null;
  clearModelDownload: () => void;
  startModelDownload: (id: string, role: "chat" | "embed") => Promise<void>;
  cancelModelDownload: () => Promise<void>;

  // theme
  theme: "midnight" | "plum" | "ocean" | "forest" | "snow" | "sunset" | "cyberpunk" | "mocha";
  setTheme: (t: AppState["theme"]) => void;

  // global preferences loaded from rust
  preferences: Record<string, string>;
  setPreferences: (p: Record<string, string>) => void;
  updatePreference: (key: string, value: string) => void;
}

export const useApp = create<AppState>((set, get) => ({
  hardware: null,
  setHardware: (h) => set({ hardware: h }),

  active_view: "home",
  setActiveView: (v) => set({ active_view: v }),

  current_session_id: null,
  current_world_id: null,
  setActiveSession: (sessionId, worldId) => {
    set({ current_session_id: sessionId, current_world_id: worldId });
    // Persist for resume on restart
    settingSet("last_session_id", sessionId).catch(() => {});
    settingSet("last_world_id", worldId).catch(() => {});
  },
  clearRuntimeState: () => {
    set({
    active_view: "home",
    current_session_id: null,
    current_world_id: null,
    selected_world_for_detail: null,
    workspace_messages: [],
    workspace_scene_images: {},
    workspace_story_state: null,
    workspace_jump_to_message: null,
    });
    // Remove stale resume pointers when the active world/session is deleted.
    settingSet("last_session_id", "").catch(() => {});
    settingSet("last_world_id", "").catch(() => {});
  },

  worlds: [],
  setWorlds: (w) => set({ worlds: w }),
  refreshWorlds: async () => {
    const list = await invoke<World[]>("world_list");
    set({ worlds: list });
  },

  selected_world_for_detail: null,
  setSelectedWorldForDetail: (w) => set({ selected_world_for_detail: w }),

  characters: [],
  setCharacters: (c) => set({ characters: c }),
  refreshCharacters: async (worldId) => {
    try {
      const list = await characterList(worldId);
      set({ characters: list });
    } catch { /* cache stays stale rather than crashing */ }
  },

  editing_world_id: null,
  setEditingWorldId: (id) => set({ editing_world_id: id }),

  // panel layout — default both open, persisted via preferences
  left_panel_open: true,
  setLeftPanelOpen: (b) => set({ left_panel_open: b }),
  right_panel_open: false,
  setRightPanelOpen: (b) => set({ right_panel_open: b }),
  workspace_messages: [],
  workspace_scene_images: {},
  workspace_story_state: null,
  workspace_jump_to_message: null,
  setWorkspaceMessages: (messages) => set({ workspace_messages: messages }),
  setWorkspaceSceneImages: (images) => set({ workspace_scene_images: images }),
  setWorkspaceStoryState: (state) => set({ workspace_story_state: state }),
  requestWorkspaceJump: (index) => set({ workspace_jump_to_message: index }),
  clearWorkspaceJump: () => set({ workspace_jump_to_message: null }),

  settings_open: false,
  setSettingsOpen: (b) => set({ settings_open: b }),
  settings_tab: "general",
  setSettingsTab: (t) => set({ settings_tab: t }),

  selected_model: null,
  setSelectedModel: (m) => set({ selected_model: m }),

  model_download: null,
  clearModelDownload: () => set({ model_download: null }),
  cancelModelDownload: async () => {
    const cur = get().model_download;
    if (cur && cur.phase === "active") {
      set({ model_download: { ...cur, phase: "cancelled", status: "Cancelled" } });
      await modelPullCancel(cur.id).catch(() => {});
    }
  },
  startModelDownload: async (id, role) => {
    if (get().model_download?.phase === "active") return;
    set({
      model_download: { id, role, status: "starting", completed: 0, total: 0, phase: "active" },
    });
    const ch = new Channel<PullProgress>();
    ch.onmessage = (p) => {
      const cur = get().model_download;
      if (!cur || cur.id !== id || cur.phase !== "active") return;
      set({
        model_download: {
          ...cur, status: p.status,
          completed: p.completed ?? cur.completed,
          total: p.total ?? cur.total,
        },
      });
    };
    try {
      await modelPull(id, ch);
      const active = get().model_download;
      if (!active || active.id !== id || active.phase !== "active") return;
      await modelSetDefault(role, id);
      const finished = get().model_download;
      if (!finished || finished.id !== id || finished.phase !== "active") return;
      set({
        model_download: {
          id, role, status: "done",
          completed: finished?.id === id ? finished.completed : 0,
          total: finished?.id === id ? finished.total : 0,
          phase: "done",
        },
      });
    } catch (e: any) {
      const failed = get().model_download;
      if (!failed || failed.id !== id || failed.phase !== "active") return;
      set({
        model_download: {
          id, role, status: "error",
          completed: failed?.id === id ? failed.completed : 0,
          total: failed?.id === id ? failed.total : 0,
          phase: "error",
          error: e?.message ?? String(e),
        },
      });
    }
  },

  theme: "midnight",
  setTheme: (t) => set({ theme: t }),

  preferences: {},
  setPreferences: (p) => set({ preferences: p }),
  updatePreference: (key, value) => {
    const hadPrevious = key in get().preferences;
    const previous = get().preferences[key];
    set((s) => ({ preferences: { ...s.preferences, [key]: value } }));
    settingSet(key, value).catch((error) => {
      console.error(error);
      set((s) => {
        const next = { ...s.preferences };
        if (hadPrevious) next[key] = previous;
        else delete next[key];
        return { preferences: next };
      });
      toast.error(i18n.t("ui.preference-not-saved", "Could not save this setting — it may reset after restart."));
    });
  },
}));
