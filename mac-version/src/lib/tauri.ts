/**
 * FPV2.0 Tauri IPC wrappers — only FPV commands + shared infrastructure.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
export { Channel };

// ---- Types ----

export interface HardwareInfo {
  chip: string;
  ram_gb: number;
  tier: string;
  recommended_chat_model: string;
}

export interface SystemStatus {
  ollama_up: boolean;
  chat_model_present: boolean;
  image_provider_ready: boolean;
}

export interface PullProgress {
  status: string;
  completed: number | null;
  total: number | null;
}

export interface ImageResult {
  image_b64: string;
  provider: string;
  model: string;
  seed: number;
  steps: number;
  cfg_scale: number;
  width: number;
  height: number;
  sampler: string;
  scheduler: string | null;
  prompt_hash: string;
}

export interface World {
  id: string;
  name: string;
  genre: string;
  description: string | null;
  system_prompt: string;
  accent_color: string | null;
  is_nsfw: boolean;
  cover_image_path: string | null;
  source: string;
  privacy_mode: "local_only" | "cloud_allowed";
}

export interface CodexEntry {
  id: string;
  world_id: string;
  title: string;
  content: string;
  triggers: string;
}

export interface Message {
  id: string;
  session_id: string;
  role: "narrator" | "user";
  content: string;
  created_at: string;
}

export interface StoryState {
  turn: number;
  location: string;
  active_goals: string[];
  open_threads: string[];
  inventory: string[];
  relationships: string[];
  facts: string[];
  conflicts: string[];
  last_scene: string;
  source: string;
  quests?: Quest[];
  inventory_items?: InventoryItem[];
  relationship_records?: RelationshipRecord[];
  pending_changes?: PendingChange[];
}

export interface Quest { id: string; title: string; status: string; description: string; }
export interface InventoryItem { name: string; quantity: number; condition: string; }
export interface RelationshipRecord { character: string; status: string; note: string; }
export interface PendingChange { id: string; kind: string; summary: string; target: string; value: string; }

export interface VisualBible {
  style: string;
  palette: string;
  character_anchors: string;
  location_anchors: string;
  negative_prompt: string;
}

export function visualBibleGet(worldId: string): Promise<VisualBible> {
  return tauriInvoke("world_visual_bible_get", { world_id: worldId });
}

export function visualBibleSet(worldId: string, visualBible: VisualBible): Promise<void> {
  return tauriInvoke("world_visual_bible_set", { world_id: worldId, visual_bible: visualBible });
}

export interface PinnedCanon {
  id: string;
  content: string;
  created_at: string;
}

export interface StoryMemoryRecord {
  message_id: string;
  content: string;
  created_at: string;
}

export interface Character {
  id: string;
  world_id: string;
  name: string;
  role: string;
  backstory: string | null;
  traits: string; // JSON array of strings
  avatar_path: string | null;
  created_at: number;
  updated_at: number;
}

// ---- FPV Story Commands ----

export function worldCreate(w: {
  name: string;
  genre: string;
  description: string | null;
  systemPrompt: string;
  accentColor: string | null;
  isNsfw: boolean;
  coverImagePath: string | null;
  source: string;
}): Promise<string> {
  return tauriInvoke("world_create", {
    w: {
      name: w.name,
      genre: w.genre,
      description: w.description,
      system_prompt: w.systemPrompt,
      accent_color: w.accentColor,
      is_nsfw: w.isNsfw,
      cover_image_path: w.coverImagePath,
      source: w.source,
    },
  });
}

export function worldList(): Promise<World[]> {
  return tauriInvoke("world_list");
}

export function worldGet(id: string): Promise<World> {
  return tauriInvoke("world_get", { id });
}

export function worldDelete(id: string): Promise<void> {
  return tauriInvoke("world_delete", { id });
}

export function sessionCreate(worldId: string): Promise<string> {
  return tauriInvoke("session_create", { world_id: worldId });
}

export function sessionListMessages(sessionId: string): Promise<Message[]> {
  return tauriInvoke("session_list_messages", { session_id: sessionId });
}

export function sessionAppendMessage(sessionId: string, role: string, content: string): Promise<string> {
  return tauriInvoke("session_append_message", { session_id: sessionId, role, content });
}

export function messageUpdate(id: string, content: string): Promise<void> {
  return tauriInvoke("message_update", { id, content });
}

export function storyStateGet(sessionId: string): Promise<StoryState> {
  return tauriInvoke("story_state_get", { session_id: sessionId });
}

export function storyStateSet(sessionId: string, storyState: StoryState): Promise<void> {
  return tauriInvoke("story_state_set", { session_id: sessionId, story_state: storyState });
}

export function storyPinnedCanonList(sessionId: string): Promise<PinnedCanon[]> {
  return tauriInvoke("story_pinned_canon_list", { session_id: sessionId });
}

export function storyPinnedCanonAdd(sessionId: string, content: string): Promise<string> {
  return tauriInvoke("story_pinned_canon_add", { session_id: sessionId, content });
}

export function storyPinnedCanonDelete(id: string): Promise<void> {
  return tauriInvoke("story_pinned_canon_delete", { id });
}

export function storyMemoryList(sessionId: string, limit = 8): Promise<StoryMemoryRecord[]> {
  return tauriInvoke("story_memory_list", { session_id: sessionId, limit });
}

export interface SessionSummary {
  id: string;
  world_id: string;
  summary: string;
  created_at: string;
  updated_at: string;
  message_count: number;
  parent_session_id: string | null;
  branch_label: string;
  is_checkpoint: boolean;
}

export function sessionListForWorld(worldId: string): Promise<SessionSummary[]> {
  return tauriInvoke("session_list_for_world", { world_id: worldId });
}

export function sessionForkFromMessage(sessionId: string, messageId: string, label: string): Promise<string> {
  return tauriInvoke("session_fork_from_message", { session_id: sessionId, message_id: messageId, label });
}

export function sessionUpdateSummary(sessionId: string, summary: string): Promise<void> {
  return tauriInvoke("session_update_summary", { session_id: sessionId, summary });
}

export function codexUpsert(entry: {
  worldId: string;
  title: string;
  content: string;
  triggers: string;
}): Promise<string> {
  return tauriInvoke("codex_upsert", {
    world_id: entry.worldId,
    title: entry.title,
    content: entry.content,
    triggers: entry.triggers,
  });
}

export function codexListForWorld(worldId: string): Promise<CodexEntry[]> {
  return tauriInvoke("codex_list_for_world", { world_id: worldId });
}

// ---- Character Commands ----

export function characterCreate(input: {
  worldId: string;
  name: string;
  role?: string;
  backstory?: string;
  traits?: string[];
  avatarPath?: string;
}): Promise<string> {
  return tauriInvoke("character_create", {
    world_id: input.worldId,
    name: input.name,
    role: input.role ?? null,
    backstory: input.backstory ?? null,
    traits: input.traits ?? null,
    avatar_path: input.avatarPath ?? null,
  });
}

export function characterList(worldId: string): Promise<Character[]> {
  return tauriInvoke("character_list", { world_id: worldId });
}

export function characterGet(id: string): Promise<Character> {
  return tauriInvoke("character_get", { id });
}

export function characterUpdate(
  id: string,
  input: {
    name?: string;
    role?: string;
    backstory?: string;
    traits?: string[];
    avatarPath?: string;
  },
): Promise<void> {
  return tauriInvoke("character_update", {
    id,
    name: input.name ?? null,
    role: input.role ?? null,
    backstory: input.backstory ?? null,
    traits: input.traits ?? null,
    avatar_path: input.avatarPath ?? null,
  });
}

export function characterDelete(id: string): Promise<void> {
  return tauriInvoke("character_delete", { id });
}

export function narrationGenerate(prompt: string, worldId: string): Promise<string> {
  return tauriInvoke("narration_generate", { prompt, world_id: worldId, request_id: crypto.randomUUID() });
}

export function narrationGenerateStream(prompt: string, worldId: string): Promise<string> {
  return tauriInvoke("narration_generate_stream", { prompt, world_id: worldId, request_id: crypto.randomUUID() });
}

export function worldGenerateCover(worldId: string): Promise<string> {
  return tauriInvoke("world_generate_cover", { world_id: worldId });
}

// ---- Image Commands ----

export interface ImageCheckResult {
  mflux_installed: boolean;
  ram_gb: number;
  local_ready: boolean;
  weights_cached: boolean | null;
}

export interface LocalModelChoice {
  id: string;
  download_gb: number;
  ready: boolean;
  large: boolean;
  steps: [number, number, number, number];
  default_steps: number;
  min_ram_gib: number;
  ram_ok: boolean;
}

export function imageLocalCheck(): Promise<ImageCheckResult> {
  return tauriInvoke("image_local_check");
}

export function imageGenerate(input: { prompt: string; style?: string; quality?: string; seed?: number; world_id?: string }): Promise<ImageResult> {
  return tauriInvoke("image_generate", { input });
}

export type ImageProviderId = "local" | "openai" | "seedream" | "hunyuan" | "cogview" | "flux" | "imagen" | "fal";

export function imageProviderGet(): Promise<ImageProviderId> {
  return tauriInvoke("image_provider_get");
}

export function imageProviderSet(provider: ImageProviderId): Promise<void> {
  return tauriInvoke("image_provider_set", { provider });
}

export function imageLocalModels(): Promise<LocalModelChoice[]> {
  return tauriInvoke("image_local_models");
}

export interface PrewarmProgress {
  phase: string;
  percent: number | null;
  message: string | null;
}

export function imageLocalPrewarm(): Promise<void> {
  return tauriInvoke("image_local_prewarm");
}

export function imageDownloadCancel(): Promise<void> {
  return tauriInvoke("image_download_cancel");
}

export function imageLocalModelGet(): Promise<string> {
  return tauriInvoke("image_local_model_get");
}

export function imageLocalModelSet(model: string): Promise<void> {
  return tauriInvoke("image_local_model_set", { model });
}

export function imageLocalModelDelete(model: string): Promise<void> {
  return tauriInvoke("image_local_model_delete", { model });
}

export function imageCloudModels(provider: ImageProviderId): Promise<string[]> {
  return tauriInvoke("image_cloud_models", { provider });
}

export function imageCloudModelGet(provider: ImageProviderId): Promise<string> {
  return tauriInvoke("image_cloud_model_get", { provider });
}

export function imageCloudModelSet(provider: ImageProviderId, model: string): Promise<void> {
  return tauriInvoke("image_cloud_model_set", { provider, model });
}

// ---- Hugging Face model browser ----

export interface HfModel {
  id: string;
  downloads: number;
  likes: number;
  tags: string[];
}

export interface HfQuant {
  quant: string;
  filename: string;
  size: number;
}

export function hfSearch(query: string): Promise<HfModel[]> {
  return tauriInvoke("hf_search", { query });
}

export function hfQuants(repo: string): Promise<HfQuant[]> {
  return tauriInvoke("hf_quants", { repo });
}

// ---- Cloud Commands ----

export function cloudListModels(provider: string): Promise<string[]> {
  return tauriInvoke("cloud_list_models", { provider });
}

// ---- Model Commands ----

export interface TagsModel {
  name: string;
  size: number;
  digest: string;
}

export interface ModelListResult {
  installed: TagsModel[];
  catalog: { id: string; kind: "chat" | "embed"; recommended: boolean; description: string }[];
  user_chat_default: string | null;
  user_embed_default: string | null;
  hardware_chat_default: string;
}

export function modelList(): Promise<ModelListResult> {
  return tauriInvoke("model_list");
}

export function modelPull(id: string, channel: Channel<PullProgress>): Promise<void> {
  return tauriInvoke("model_pull", { args: { model: id }, channel });
}

export function modelPullCancel(id: string): Promise<void> {
  return tauriInvoke("model_pull_cancel", { args: { model: id } });
}

export function modelDelete(model: string): Promise<void> {
  return tauriInvoke("model_delete", { args: { model } });
}

export function modelSetDefault(role: string, model: string): Promise<void> {
  return tauriInvoke("model_set_default", { args: { role, model } });
}

export function modelPrewarmActive(): Promise<void> {
  return tauriInvoke("model_prewarm_active");
}

export function modelUnloadActive(): Promise<void> {
  return tauriInvoke("model_unload_active");
}

export function codexStatus(): Promise<boolean> {
  return tauriInvoke("codex_status");
}

export function codexLogin(): Promise<void> {
  return tauriInvoke("codex_login");
}

export function codexModelGet(): Promise<string> {
  return tauriInvoke("codex_model_get");
}

export function codexModelSet(model: string): Promise<void> {
  return tauriInvoke("codex_model_set", { model });
}

export function narrationCancel(requestId: string): Promise<void> {
  return tauriInvoke("narration_cancel", { request_id: requestId });
}

// ---- BYOK Commands ----

export function byokSave(provider: string, apiKey: string, baseUrl?: string): Promise<void> {
  return tauriInvoke("byok_save", { args: { provider, api_key: apiKey, base_url: baseUrl } });
}

export function byokDelete(provider: string): Promise<void> {
  return tauriInvoke("byok_delete", { args: { provider } });
}

export function byokList(): Promise<{ providers: string[] }> {
  return tauriInvoke("byok_list");
}

export function byokGetBaseUrl(): Promise<string | null> {
  return tauriInvoke("byok_get_base_url");
}

// ---- Settings Commands ----

export function settingGet(key: string): Promise<string | null> {
  return tauriInvoke("setting_get", { key });
}

export function settingGetAll(): Promise<Record<string, string>> {
  return tauriInvoke("setting_get_all");
}

export function settingSet(key: string, value: string): Promise<void> {
  return tauriInvoke("setting_set", { key, value });
}

// ---- System Commands ----

/// Mirrors the `SystemReady` struct in `src-tauri/src/commands/system.rs`.
/// This used to be typed `Promise<boolean>`, which made every caller's
/// truthiness check pass on `{ ollama_ready: false }` — the readiness
/// guard silently never waited. Keep this shape in step with the Rust struct.
export function systemReadyGet(): Promise<{ ollama_ready: boolean }> {
  return tauriInvoke("system_ready_get");
}

export function systemEnergyGet(): Promise<{ on_ac_power: boolean; battery_percent: number | null; thermally_constrained: boolean }> {
  return tauriInvoke("system_energy_get");
}

export function systemStatus(): Promise<SystemStatus> {
  return tauriInvoke("system_status");
}

// ---- Hardware Commands ----

export function hardwareDetect(): Promise<HardwareInfo> {
  return tauriInvoke("hardware_detect");
}

// ---- Advanced Commands ----

export function logsPath(): Promise<string> {
  return tauriInvoke("logs_path");
}

export function appLogPath(): Promise<string> {
  return tauriInvoke("app_log_path");
}

export function diagnosticsWrite(): Promise<string> {
  return tauriInvoke("diagnostics_write");
}

export function logRendererCrash(
  message: string,
  stack = "(no stack)",
  componentStack = "(no component stack)",
): Promise<void> {
  return tauriInvoke("log_renderer_crash", {
    message,
    stack,
    component_stack: componentStack,
  });
}

export function resetAllData(): Promise<void> {
  return tauriInvoke("reset_all_data");
}

// ---- Update Safety Commands ----

export interface PreUpdateCheckResult {
  ok: boolean;
  reason: string;
  data_size_bytes: number;
}

export function preUpdateCheck(): Promise<PreUpdateCheckResult> {
  return tauriInvoke("pre_update_check");
}

export function backupAppData(): Promise<void> {
  return tauriInvoke("backup_app_data");
}

export function postUpdateFinalize(): Promise<{ was_post_update_boot: boolean; warning: string | null }> {
  return tauriInvoke("post_update_finalize");
}

// ---- Window Utilities ----

export function startWindowDrag(): void {
  // Tauri 2 drag region — handled by data-tauri-drag-region attribute
}
